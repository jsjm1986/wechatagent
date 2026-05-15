import {
  Activity,
  Bot,
  Clock3,
  MessageSquareText,
  RefreshCw,
  Search,
  SendHorizonal,
  SquarePen,
  UserRoundCheck,
  UsersRound
} from "lucide-react";
import { FormEvent, useEffect, useMemo, useState } from "react";

type AgentStatus = "normal" | "managed";

type Account = {
  id: string;
  alias: string;
  displayName: string;
  appId?: string;
  wxid?: string;
  nickName?: string;
  online: boolean;
};

type AgentProfile = {
  summary: string;
  interests: string[];
  communicationStyle: string;
  operationGoal: string;
};

type Contact = {
  id: string;
  wxid: string;
  nickname?: string;
  remark?: string;
  alias?: string;
  agentStatus: AgentStatus;
  humanProfileNote?: string;
  agentProfile?: AgentProfile;
  memorySummary?: string;
  updatedAt: string;
};

type Message = {
  id: string;
  direction: "inbound" | "outbound";
  content: string;
  createdAt?: string;
};

type EventItem = {
  id: string;
  contactWxid?: string;
  kind: string;
  status: string;
  summary: string;
  createdAt?: string;
};

type TaskItem = {
  id: string;
  contactWxid: string;
  kind: string;
  runAt?: string;
  content: string;
  status: string;
  error?: string;
};

const api = {
  async get<T>(url: string): Promise<T> {
    const response = await fetch(url);
    if (!response.ok) throw new Error(await response.text());
    return response.json();
  },
  async post<T>(url: string, body?: unknown): Promise<T> {
    const response = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: body ? JSON.stringify(body) : undefined
    });
    if (!response.ok) throw new Error(await response.text());
    return response.json();
  },
  async put<T>(url: string, body: unknown): Promise<T> {
    const response = await fetch(url, {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body)
    });
    if (!response.ok) throw new Error(await response.text());
    return response.json();
  }
};

export function App() {
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [selected, setSelected] = useState<Contact | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [events, setEvents] = useState<EventItem[]>([]);
  const [tasks, setTasks] = useState<TaskItem[]>([]);
  const [query, setQuery] = useState("");
  const [importQuery, setImportQuery] = useState("");
  const [profileNote, setProfileNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const managedCount = useMemo(
    () => contacts.filter((contact) => contact.agentStatus === "managed").length,
    [contacts]
  );

  async function loadAll() {
    setError("");
    const [accountData, contactData, eventData, taskData] = await Promise.all([
      api.get<{ items: Account[] }>("/api/accounts"),
      api.get<{ items: Contact[] }>(`/api/contacts${query ? `?q=${encodeURIComponent(query)}` : ""}`),
      api.get<{ items: EventItem[] }>("/api/events"),
      api.get<{ items: TaskItem[] }>("/api/tasks")
    ]);
    setAccounts(accountData.items);
    setContacts(contactData.items);
    setEvents(eventData.items);
    setTasks(taskData.items);
    if (selected) {
      const refreshed = contactData.items.find((item) => item.id === selected.id);
      setSelected(refreshed ?? null);
    }
  }

  async function loadMessages(contact: Contact) {
    setSelected(contact);
    setProfileNote(contact.humanProfileNote ?? "");
    const data = await api.get<{ items: Message[] }>(`/api/conversations/${contact.id}/messages`);
    setMessages(data.items.reverse());
  }

  async function syncAccounts() {
    await run(async () => {
      await api.post("/api/accounts/sync");
      await loadAll();
    });
  }

  async function importContacts(event: FormEvent) {
    event.preventDefault();
    if (!importQuery.trim()) return;
    await run(async () => {
      const data = await api.post<{ items: Contact[] }>("/api/contacts/search-import", {
        query: importQuery
      });
      setImportQuery("");
      await loadAll();
      if (data.items[0]) await loadMessages(data.items[0]);
    });
  }

  async function enableAgent() {
    if (!selected || !profileNote.trim()) return;
    await run(async () => {
      const data = await api.post<{ item: Contact }>(`/api/contacts/${selected.id}/enable-agent`, {
        humanProfileNote: profileNote
      });
      setSelected(data.item);
      await loadAll();
    });
  }

  async function saveProfileNote() {
    if (!selected) return;
    await run(async () => {
      const data = await api.put<{ item: Contact }>(`/api/contacts/${selected.id}/profile-note`, {
        humanProfileNote: profileNote
      });
      setSelected(data.item);
      await loadAll();
    });
  }

  async function disableAgent() {
    if (!selected) return;
    await run(async () => {
      const data = await api.post<{ item: Contact }>(`/api/contacts/${selected.id}/disable-agent`);
      setSelected(data.item);
      setProfileNote(data.item.humanProfileNote ?? "");
      await loadAll();
    });
  }

  async function run(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  useEffect(() => {
    void loadAll().catch((err) => setError(err instanceof Error ? err.message : String(err)));
  }, []);

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="brandMark">
            <Bot size={20} />
          </div>
          <div>
            <strong>WechatAgent</strong>
            <span>私聊运营控制台</span>
          </div>
        </div>

        <nav>
          <a href="#contacts">
            <UsersRound size={16} />
            好友纳管
          </a>
          <a href="#conversation">
            <MessageSquareText size={16} />
            会话画像
          </a>
          <a href="#ops">
            <Activity size={16} />
            运营日志
          </a>
        </nav>
      </aside>

      <main>
        <header className="topline">
          <div>
            <p>Agent Operations</p>
            <h1>只运营被纳管的好友</h1>
          </div>
          <div className="actions">
            <button onClick={() => void syncAccounts()} disabled={busy}>
              <RefreshCw size={16} />
              同步账号
            </button>
            <button onClick={() => void loadAll()} disabled={busy}>
              <RefreshCw size={16} />
              刷新
            </button>
          </div>
        </header>

        {error && <div className="error">{error}</div>}

        <section className="metrics">
          <div className="metric">
            <span>微信账号</span>
            <strong>{accounts.length}</strong>
          </div>
          <div className="metric">
            <span>已纳管好友</span>
            <strong>{managedCount}</strong>
          </div>
          <div className="metric">
            <span>跟进任务</span>
            <strong>{tasks.length}</strong>
          </div>
        </section>

        <section id="contacts" className="workspace">
          <div className="pane contactPane">
            <div className="paneHead">
              <div>
                <span>Contacts</span>
                <h2>好友列表</h2>
              </div>
            </div>

            <form className="searchRow" onSubmit={importContacts}>
              <label>
                <Search size={15} />
                <input
                  value={importQuery}
                  onChange={(event) => setImportQuery(event.target.value)}
                  placeholder="搜索并导入好友，例如 AI应用开发"
                />
              </label>
              <button type="submit" disabled={busy || !importQuery.trim()}>
                导入
              </button>
            </form>

            <label className="filter">
              <Search size={15} />
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                onBlur={() => void loadAll()}
                placeholder="过滤已导入好友"
              />
            </label>

            <div className="contactList">
              {contacts.map((contact) => (
                <button
                  key={contact.id}
                  className={selected?.id === contact.id ? "contact selected" : "contact"}
                  onClick={() => void loadMessages(contact)}
                >
                  <span className={contact.agentStatus === "managed" ? "dot managed" : "dot"} />
                  <div>
                    <strong>{contact.remark || contact.nickname || contact.wxid}</strong>
                    <small>{contact.alias || contact.wxid}</small>
                  </div>
                  <em>{contact.agentStatus === "managed" ? "Agent" : "普通"}</em>
                </button>
              ))}
            </div>
          </div>

          <div id="conversation" className="pane detailPane">
            {selected ? (
              <>
                <div className="paneHead">
                  <div>
                    <span>Profile</span>
                    <h2>{selected.remark || selected.nickname || selected.wxid}</h2>
                  </div>
                  <div className="statusPill">
                    <UserRoundCheck size={15} />
                    {selected.agentStatus === "managed" ? "Agent运营中" : "普通好友"}
                  </div>
                </div>

                <div className="profileEditor">
                  <label>
                    <span>运营备注</span>
                    <textarea
                      value={profileNote}
                      onChange={(event) => setProfileNote(event.target.value)}
                      placeholder="例如：老客户，做知识付费，对 AI 私域运营感兴趣，沟通要直接，不要太营销。"
                    />
                  </label>
                  <div className="buttonRow">
                    {selected.agentStatus === "managed" ? (
                      <>
                        <button onClick={() => void saveProfileNote()} disabled={busy}>
                          <SquarePen size={16} />
                          保存并重建画像
                        </button>
                        <button className="secondary" onClick={() => void disableAgent()} disabled={busy}>
                          停止运营
                        </button>
                      </>
                    ) : (
                      <button onClick={() => void enableAgent()} disabled={busy || !profileNote.trim()}>
                        <SendHorizonal size={16} />
                        加入 Agent 运营
                      </button>
                    )}
                  </div>
                </div>

                <div className="profileGrid">
                  <div>
                    <span>画像摘要</span>
                    <p>{selected.agentProfile?.summary || "尚未生成"}</p>
                  </div>
                  <div>
                    <span>沟通风格</span>
                    <p>{selected.agentProfile?.communicationStyle || "尚未生成"}</p>
                  </div>
                  <div>
                    <span>运营目标</span>
                    <p>{selected.agentProfile?.operationGoal || "尚未生成"}</p>
                  </div>
                  <div>
                    <span>长期记忆</span>
                    <p>{selected.memorySummary || "暂无"}</p>
                  </div>
                </div>

                <div className="messageList">
                  {messages.map((message) => (
                    <div key={message.id} className={`message ${message.direction}`}>
                      <p>{message.content}</p>
                      <span>{formatTime(message.createdAt)}</span>
                    </div>
                  ))}
                </div>
              </>
            ) : (
              <div className="emptyState">
                <MessageSquareText size={34} />
                <p>选择一个好友查看画像和会话。</p>
              </div>
            )}
          </div>
        </section>

        <section id="ops" className="opsGrid">
          <div className="pane">
            <div className="paneHead">
              <div>
                <span>Events</span>
                <h2>运营日志</h2>
              </div>
            </div>
            <table>
              <tbody>
                {events.map((event) => (
                  <tr key={event.id}>
                    <td>{event.kind}</td>
                    <td>{event.summary}</td>
                    <td>{event.status}</td>
                    <td>{formatTime(event.createdAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          <div className="pane">
            <div className="paneHead">
              <div>
                <span>Tasks</span>
                <h2>跟进任务</h2>
              </div>
              <Clock3 size={18} />
            </div>
            <table>
              <tbody>
                {tasks.map((task) => (
                  <tr key={task.id}>
                    <td>{task.status}</td>
                    <td>{task.content}</td>
                    <td>{formatTime(task.runAt)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      </main>
    </div>
  );
}

function formatTime(value?: string) {
  if (!value) return "-";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(value));
}

