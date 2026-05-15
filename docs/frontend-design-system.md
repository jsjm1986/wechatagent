# Frontend Design System

WechatAgent is a new AI operations product for enterprise users. The interface must feel premium, calm, trustworthy, and unmistakably AI-enabled without becoming decorative or complex.

## Visual Thesis

Use a white enterprise console language: white navigation, cold gray workspace, graphite typography, one AI-blue action color, and a restrained teal AI-status accent. AI should appear through status language, subtle luminous states, and precise system behavior, not heavy gradients or sci-fi decoration.

## Layout System

The application uses a stable channel shell:

```text
white sidebar channel navigation
main workspace
  page header
  current channel content
  optional sub-tabs
```

Core CSS tokens live in `frontend/src/styles.css`:

```css
--sidebar-width: 264px;
--page-x: 30px;
--page-y: 24px;
--section-gap: 18px;
--panel-pad: 18px;
--control-h: 38px;
--row-h: 62px;
```

Do not create long pages that stack every product module. Add new capability as either:

- a new sidebar channel, when it is a first-class product area
- a sub-tab inside an existing channel, when it is part of that workflow
- a compact summary card on Overview, when it is only an entry point or status

## Navigation

Use white sidebar navigation by default.

- Sidebar contains brand, AI status, and first-level channels.
- Channels are product areas, not anchor links into a long page.
- Main content renders only the active channel.
- Sub-tabs classify content inside the active channel.

Current channel model:

```text
Overview
Contacts
Agent Profile
Operations
```

## Hierarchy

Use four levels only:

1. App shell: white sidebar and main workspace.
2. Channel header: current section title and primary actions.
3. Panel or summary card: one operational job or one entry point.
4. Rows, forms, messages, and table entries.

Avoid nested panels. If a feature needs secondary content, use sub-tabs, a divider, a table row, or a compact summary card.

## Color

Primary palette:

```css
--bg: #f6f8fb;
--surface: #ffffff;
--ink: #111827;
--muted: #64748b;
--accent: #2563eb;
--ai: #0f766e;
```

Rules:

- White and near-white surfaces dominate.
- Blue is only for primary actions, selection, and active UI.
- Teal is only for AI state or managed/active signals.
- Danger/success colors are semantic only.
- No dark navigation, decorative purple gradients, neon effects, or large tinted backgrounds.

## Typography

Use the existing font stack only:

```css
"IBM Plex Sans", "Aptos", "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif
```

Rules:

- Page title: 28-40px.
- Panel title: 18px.
- Body, rows, and tables: 13px.
- Metadata and system labels: 10.5-12px.
- Letter spacing must be `0` for ordinary text.
- Do not scale routine UI text with viewport width.

## Component Rules

Summary cards:

- Allowed on Overview and as module entry points.
- Must be clickable or communicate a key status.
- Do not use card grids as the whole product structure.

Panels:

- One panel = one operational job.
- Prefer panel + sub-tabs over vertically stacking multiple panels.
- Avoid putting cards inside panels.

Navigation:

- Sidebar channels are first-level product areas.
- Sub-tabs are second-level workflow states.
- Do not add a third persistent navigation level.

Lists:

- Use fixed row height via `--row-h`.
- Long names, wxids, aliases, and task content must truncate or wrap within the region.
- Selection uses soft blue fill, not heavy borders.

Forms:

- Inputs are full width.
- Textarea is reserved for meaningful operator input.
- All focus states use the same accent ring.

Tables:

- Use for logs, tasks, and status history.
- Keep rows compact and scannable.
- Do not wrap tables in extra cards.

## Responsive Rules

At widths below `860px`:

- Sidebar becomes a top block.
- Channel navigation becomes horizontal scroll.
- Overview cards collapse to one column.
- Panels and profile grids collapse to one column.
- Sub-tabs may scroll horizontally.

Do not create mobile-only content unless the desktop content cannot be made readable.

## Extension Checklist

Before adding a new feature:

- Is it a channel, a sub-tab, or an overview entry card?
- Does the active channel still fit without becoming a long page?
- Does it use existing tokens for spacing, row height, panel padding, and controls?
- Is the AI expression limited to status, language, and subtle state?
- Are there no nested cards or third-level persistent navigation?
- Can an operator understand the screen by scanning channel title, sub-tabs, labels, and statuses?

