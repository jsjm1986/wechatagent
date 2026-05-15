import {
  Activity,
  Bot,
  CheckCircle2,
  Clock3,
  LayoutDashboard,
  MessageSquareText,
  RefreshCw,
  Search,
  SendHorizonal,
  Sparkles,
  SquarePen,
  UserRoundCheck,
  UsersRound
} from "lucide-react";
import { FormEvent, useEffect, useMemo, useState } from "react";

type AgentStatus = "normal" | "managed";
type Channel = "overview" | "contacts" | "profile" | "operations";
type ContactTab = "all" | "managed" | "normal";
type ProfileTab = "note" | "profile" | "messages";
type OpsTab = "tasks" | "events";

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

const channels: Array<{ id: Channel; label: string; caption: string; icon: typeof LayoutDashboard }> = [
  { id: "overview", label: "概览", caption: "运行状态", icon: LayoutDashboard },
  { id: "contacts", label: "好友运营", caption: "纳管列表", icon: UsersRound },
  { id: "profile", label: "Agent 画像", caption: "策略记忆", icon: Sparkles },
  { id: "operations", label: "任务与日志", caption: "执行记录", icon: Activity }
];

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
  const [activeChannel, setActiveChannel] = useState<Channel>("overview");
  const [contactTab, setContactTab] = useState<ContactTab>("all");
  const [profileTab, setProfileTab] = useState<ProfileTab>("note");
  const [opsTab, setOpsTab] = useState<OpsTab>("tasks");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const managedCount = useMemo(
    () => contacts.filter((contact) => contact.agentStatus === "managed").length,
    [contacts]
  );
  const normalCount = contacts.length - managedCount;
  const onlineCount = accounts.filter((account) => account.online).length;
  const filteredContacts = useMemo(() => {
    if (contactTab === "managed") {
      return contacts.filter((contact) => contact.agentStatus === "managed");
    }
    if (contactTab === "normal") {
      return contacts.filter((contact) => contact.agentStatus === "normal");
    }
    return contacts;
  }, [contacts, contactTab]);
  const latestEvent = events[0];
  const pendingTasks = tasks.filter((task) => task.status === "pending").length;

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

  async function openContact(contact: Contact, channel: Channel = "profile") {
    await loadMessages(contact);
    setActiveChannel(channel);
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
      if (data.items[0]) {
        await openContact(data.items[0], "profile");
      }
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
            <Bot size={19} />
          </div>
          <div>
            <strong>WechatAgent</strong>
            <span>AI OPERATIONS SUITE</span>
          </div>
        </div>

        <div className="aiStatus">
          <span />
          <div>
            <strong>AI Agent Ready</strong>
            <small>{managedCount} managed contacts</small>
          </div>
        </div>

        <nav className="channelNav">
          {channels.map((channel) => {
            const Icon = channel.icon;
            return (
              <button
                key={channel.id}
                className={activeChannel === channel.id ? "channel active" : "channel"}
                onClick={() => setActiveChannel(channel.id)}
              >
                <Icon size={16} />
                <span>
                  <strong>{channel.label}</strong>
                  <small>{channel.caption}</small>
                </span>
              </button>
            );
          })}
        </nav>
      </aside>

      <main>
        <header className="topline">
          <div>
            <p>Managed WeChat Operations</p>
            <h1>{channelTitle(activeChannel)}</h1>
          </div>
          <div className="actions">
            <button onClick={() => void syncAccounts()} disabled={busy}>
              <RefreshCw size={16} />
              同步账号
            </button>
            <button className="secondary" onClick={() => void loadAll()} disabled={busy}>
              <RefreshCw size={16} />
              刷新
            </button>
          </div>
        </header>

        {error && <div className="error">{error}</div>}

        {activeChannel === "overview" && (
          <OverviewView
            accounts={accounts}
            contacts={contacts}
            managedCount={managedCount}
            normalCount={normalCount}
            onlineCount={onlineCount}
            pendingTasks={pendingTasks}
            latestEvent={latestEvent}
            onOpenChannel={setActiveChannel}
          />
        )}

        {activeChannel === "contacts" && (
          <ContactsView
            busy={busy}
            contactTab={contactTab}
            contacts={filteredContacts}
            importQuery={importQuery}
            query={query}
            totalCount={contacts.length}
            managedCount={managedCount}
            normalCount={normalCount}
            selected={selected}
            onContactTab={setContactTab}
            onImport={importContacts}
            onImportQuery={setImportQuery}
            onLoadAll={() => void loadAll()}
            onOpenContact={(contact) => void openContact(contact, "profile")}
            onQuery={setQuery}
          />
        )}

        {activeChannel === "profile" && (
          <ProfileView
            busy={busy}
            messages={messages}
            profileNote={profileNote}
            profileTab={profileTab}
            selected={selected}
            onDisableAgent={() => void disableAgent()}
            onEnableAgent={() => void enableAgent()}
            onProfileNote={setProfileNote}
            onProfileTab={setProfileTab}
            onSaveProfileNote={() => void saveProfileNote()}
            onShowContacts={() => setActiveChannel("contacts")}
          />
        )}

        {activeChannel === "operations" && (
          <OperationsView events={events} opsTab={opsTab} tasks={tasks} onOpsTab={setOpsTab} />
        )}
      </main>
    </div>
  );
}

function OverviewView({
  accounts,
  contacts,
  managedCount,
  normalCount,
  onlineCount,
  pendingTasks,
  latestEvent,
  onOpenChannel
}: {
  accounts: Account[];
  contacts: Contact[];
  managedCount: number;
  normalCount: number;
  onlineCount: number;
  pendingTasks: number;
  latestEvent?: EventItem;
  onOpenChannel: (channel: Channel) => void;
}) {
  return (
    <section className="overviewGrid">
      <button className="summaryCard primary" onClick={() => onOpenChannel("contacts")}>
        <span>Managed</span>
        <strong>{managedCount}</strong>
        <small>Agent 运营好友</small>
      </button>
      <button className="summaryCard" onClick={() => onOpenChannel("contacts")}>
        <span>Contacts</span>
        <strong>{contacts.length}</strong>
        <small>{normalCount} 普通好友</small>
      </button>
      <button className="summaryCard" onClick={() => onOpenChannel("overview")}>
        <span>Accounts</span>
        <strong>
          {onlineCount}/{accounts.length}
        </strong>
        <small>在线账号</small>
      </button>
      <button className="summaryCard" onClick={() => onOpenChannel("operations")}>
        <span>Pending</span>
        <strong>{pendingTasks}</strong>
        <small>待执行任务</small>
      </button>

      <section className="widePanel">
        <div className="panelHead">
          <div>
            <span>Operating Model</span>
            <h2>Agent 运营边界</h2>
          </div>
          <Sparkles size={18} />
        </div>
        <div className="principleGrid">
          <div>
            <strong>白名单运营</strong>
            <p>只有加入 Agent 运营的好友会自动回复，普通好友只保留基础记录。</p>
          </div>
          <div>
            <strong>画像驱动</strong>
            <p>每个 managed 好友都有独立运营备注、画像摘要、长期记忆。</p>
          </div>
          <div>
            <strong>事件可追踪</strong>
            <p>回复、任务、失败和 MCP 调用都进入日志链路，便于复盘。</p>
          </div>
        </div>
      </section>

      <section className="sidePanel">
        <div className="panelHead">
          <div>
            <span>Last Event</span>
            <h2>最近事件</h2>
          </div>
          <Activity size={18} />
        </div>
        {latestEvent ? (
          <div className="eventPreview">
            <strong>{latestEvent.kind}</strong>
            <p>{latestEvent.summary}</p>
            <small>{formatTime(latestEvent.createdAt)}</small>
          </div>
        ) : (
          <EmptyInline text="暂无运营事件" />
        )}
      </section>
    </section>
  );
}

function ContactsView({
  busy,
  contactTab,
  contacts,
  importQuery,
  managedCount,
  normalCount,
  query,
  selected,
  totalCount,
  onContactTab,
  onImport,
  onImportQuery,
  onLoadAll,
  onOpenContact,
  onQuery
}: {
  busy: boolean;
  contactTab: ContactTab;
  contacts: Contact[];
  importQuery: string;
  managedCount: number;
  normalCount: number;
  query: string;
  selected: Contact | null;
  totalCount: number;
  onContactTab: (tab: ContactTab) => void;
  onImport: (event: FormEvent) => void;
  onImportQuery: (value: string) => void;
  onLoadAll: () => void;
  onOpenContact: (contact: Contact) => void;
  onQuery: (value: string) => void;
}) {
  return (
    <section className="workspace single">
      <div className="panel">
        <div className="panelHead">
          <div>
            <span>Contacts</span>
            <h2>好友运营池</h2>
          </div>
          <div className="segmented">
            <button className={contactTab === "all" ? "active" : ""} onClick={() => onContactTab("all")}>
              全部 {totalCount}
            </button>
            <button className={contactTab === "managed" ? "active" : ""} onClick={() => onContactTab("managed")}>
              Agent {managedCount}
            </button>
            <button className={contactTab === "normal" ? "active" : ""} onClick={() => onContactTab("normal")}>
              普通 {normalCount}
            </button>
          </div>
        </div>

        <div className="toolbar">
          <form className="searchRow" onSubmit={onImport}>
            <label>
              <Search size={15} />
              <input
                value={importQuery}
                onChange={(event) => onImportQuery(event.target.value)}
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
              onChange={(event) => onQuery(event.target.value)}
              onBlur={onLoadAll}
              placeholder="过滤已导入好友"
            />
          </label>
        </div>

        <div className="contactList tableLike">
          {contacts.map((contact) => (
            <button
              key={contact.id}
              className={selected?.id === contact.id ? "contact selected" : "contact"}
              onClick={() => onOpenContact(contact)}
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
    </section>
  );
}

function ProfileView({
  busy,
  messages,
  profileNote,
  profileTab,
  selected,
  onDisableAgent,
  onEnableAgent,
  onProfileNote,
  onProfileTab,
  onSaveProfileNote,
  onShowContacts
}: {
  busy: boolean;
  messages: Message[];
  profileNote: string;
  profileTab: ProfileTab;
  selected: Contact | null;
  onDisableAgent: () => void;
  onEnableAgent: () => void;
  onProfileNote: (value: string) => void;
  onProfileTab: (tab: ProfileTab) => void;
  onSaveProfileNote: () => void;
  onShowContacts: () => void;
}) {
  if (!selected) {
    return (
      <section className="panel emptyPanel">
        <EmptyInline text="先从好友运营频道选择一个好友。" />
        <button onClick={onShowContacts}>
          <UsersRound size={16} />
          前往好友运营
        </button>
      </section>
    );
  }

  return (
    <section className="panel">
      <div className="panelHead">
        <div>
          <span>Selected Agent Profile</span>
          <h2>{selected.remark || selected.nickname || selected.wxid}</h2>
        </div>
        <div className="statusPill">
          <UserRoundCheck size={15} />
          {selected.agentStatus === "managed" ? "AI Active" : "普通好友"}
        </div>
      </div>

      <div className="subTabs">
        <button className={profileTab === "note" ? "active" : ""} onClick={() => onProfileTab("note")}>
          运营备注
        </button>
        <button className={profileTab === "profile" ? "active" : ""} onClick={() => onProfileTab("profile")}>
          画像摘要
        </button>
        <button className={profileTab === "messages" ? "active" : ""} onClick={() => onProfileTab("messages")}>
          会话记录
        </button>
      </div>

      {profileTab === "note" && (
        <div className="profileEditor">
          <label>
            <span>运营备注</span>
            <textarea
              value={profileNote}
              onChange={(event) => onProfileNote(event.target.value)}
              placeholder="例如：老客户，做知识付费，对 AI 私域运营感兴趣，沟通要直接，不要太营销。"
            />
          </label>
          <div className="buttonRow">
            {selected.agentStatus === "managed" ? (
              <>
                <button onClick={onSaveProfileNote} disabled={busy}>
                  <SquarePen size={16} />
                  保存并重建画像
                </button>
                <button className="secondary" onClick={onDisableAgent} disabled={busy}>
                  停止运营
                </button>
              </>
            ) : (
              <button onClick={onEnableAgent} disabled={busy || !profileNote.trim()}>
                <SendHorizonal size={16} />
                加入 Agent 运营
              </button>
            )}
          </div>
        </div>
      )}

      {profileTab === "profile" && (
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
      )}

      {profileTab === "messages" && (
        <div className="messageList">
          {messages.map((message) => (
            <div key={message.id} className={`message ${message.direction}`}>
              <p>{message.content}</p>
              <span>{formatTime(message.createdAt)}</span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

function OperationsView({
  events,
  opsTab,
  tasks,
  onOpsTab
}: {
  events: EventItem[];
  opsTab: OpsTab;
  tasks: TaskItem[];
  onOpsTab: (tab: OpsTab) => void;
}) {
  return (
    <section className="panel">
      <div className="panelHead">
        <div>
          <span>Operations</span>
          <h2>任务与日志</h2>
        </div>
        <Clock3 size={18} />
      </div>

      <div className="subTabs">
        <button className={opsTab === "tasks" ? "active" : ""} onClick={() => onOpsTab("tasks")}>
          跟进任务
        </button>
        <button className={opsTab === "events" ? "active" : ""} onClick={() => onOpsTab("events")}>
          运营事件
        </button>
      </div>

      {opsTab === "tasks" && (
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
      )}

      {opsTab === "events" && (
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
      )}
    </section>
  );
}

function EmptyInline({ text }: { text: string }) {
  return (
    <div className="emptyState">
      <MessageSquareText size={30} />
      <p>{text}</p>
    </div>
  );
}

function channelTitle(channel: Channel) {
  switch (channel) {
    case "overview":
      return "AI 运营概览";
    case "contacts":
      return "好友运营";
    case "profile":
      return "Agent 画像";
    case "operations":
      return "任务与日志";
  }
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
