# Frontend Design System

This product is an enterprise operations console. The UI must stay restrained, dense, and predictable as features grow.

## Visual Thesis

WechatAgent uses a serious enterprise control-room language: graphite navigation, cold white work surfaces, thin structural lines, one cobalt action color, and compact typography. The interface should feel operational, not promotional.

## Layout System

Use a stable application shell:

```text
252px sidebar
remaining workspace
topline header
metric strip
primary workspace grid
secondary operations grid
```

Core CSS tokens live in `frontend/src/styles.css`:

```css
--sidebar-width: 252px;
--page-x: 28px;
--page-y: 24px;
--section-gap: 18px;
--pane-pad: 16px;
--control-h: 36px;
--row-h: 58px;
```

Do not introduce page-specific gutters, random max-widths, or nested card stacks. New pages should reuse the shell and either:

- a two-column workspace: `390px minmax(0, 1fr)`
- a two-column operations grid: `minmax(0, 1fr) minmax(0, 1fr)`
- a single-column mobile flow below `980px`

## Hierarchy

Use four visual levels only:

1. App shell: dark sidebar and main workspace.
2. Page orientation: topline title and primary actions.
3. Operational regions: bordered panes with one header and one job.
4. Rows, fields, messages, and table entries.

Avoid adding another visual level unless the product model truly needs it. If a new element requires a nested panel inside a pane, first try a divider, table row, segmented area, or inline section.

## Typography

Use the existing font stack only:

```css
"IBM Plex Sans", "Aptos", "Segoe UI", "PingFang SC", "Microsoft YaHei", sans-serif
```

Rules:

- Page title: 26-38px, one line if possible.
- Pane title: 18px.
- Body and table text: 13px.
- Metadata and labels: 11-12px uppercase where appropriate.
- Do not use negative letter spacing.
- Do not scale font size with viewport width except the existing page title clamp.

## Color

Use one accent color:

```css
--accent: #1d4ed8;
```

Allowed semantic colors:

- success: managed/online/complete
- danger: errors/destructive states
- muted: metadata/inactive states

Do not add decorative gradients, multi-accent palettes, purple-blue marketing gradients, or large tinted backgrounds. Data density and alignment should create the premium feeling.

## Component Rules

Buttons:

- Primary button only for direct committed actions.
- Secondary button for safe reversible actions.
- Icon + text for commands.
- Keep height at `--control-h`.

Panes:

- One pane = one operational job.
- Pane headers must include a short English scope label and a Chinese working title.
- Avoid cards inside panes. Use dividers and rows.

Lists:

- Use fixed row height via `--row-h`.
- Contact rows must truncate names and identifiers.
- Selection uses `--accent-soft`, not heavy borders.

Tables:

- Tables are for logs/tasks/status.
- Keep rows compact.
- Do not place tables inside additional card wrappers.

Forms:

- Inputs use full width.
- Textarea is only for substantial operator input.
- Focus state is the same across fields: accent border plus subtle ring.

## Responsive Rules

At widths below `980px`:

- Sidebar becomes normal flow.
- Workspace, metric strip, profile grid, and operations grid collapse to one column.
- Page padding becomes `18px`.
- Do not introduce separate mobile-only content unless the desktop content cannot be made readable.

Text must never overlap or require horizontal scrolling. Long names, wxids, aliases, task content, and logs must truncate or wrap within their region.

## Extension Checklist

Before adding a new screen or feature:

- Does it fit the existing shell?
- Does the section have exactly one job?
- Can it use an existing pane, row, table, or field pattern?
- Are all dimensions tied to existing tokens?
- Does mobile collapse follow the `980px` rule?
- Are there no nested cards or decorative panels?
- Can an operator understand the page by scanning headings, labels, numbers, and statuses?

