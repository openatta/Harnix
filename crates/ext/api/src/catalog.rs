//! Every seam the interface is assembled out of, and what each one costs.
//!
//! This is the source. The table in `docs/contribution_points.md` is rendered
//! from it, and a test compares the two — see the module docs.
//!
//! Engine and host extension points are *not* here, and not because they were
//! forgotten: extending the agent is Agix's, and a copy of its catalog
//! kept on this side would be a second copy to drift.

use std::fmt::Write as _;

use crate::ui::PointDecl;

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointKind {
    /// Contribute a named, ordered, removable thing.
    Register,
    /// Fold a stream into derived state.
    Fold,
    /// Draw something.
    Render,
}

/// The order of magnitude a point fires at. This is a constraint, not
/// decoration: it decides whether a point is open, and to what.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    PerProcess,
    PerSession,
    PerTurn,
    /// When a block's state changes — a tool starts, finishes, fails. Tens
    /// per turn. Explicitly *not* per streaming delta.
    PerBlockChange,
    /// When the user's selection or a panel's input changes.
    PerInteraction,
}

impl PointKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PointKind::Register => "注册",
            PointKind::Fold => "折叠",
            PointKind::Render => "呈现",
        }
    }
}

impl Frequency {
    pub fn as_str(&self) -> &'static str {
        match self {
            Frequency::PerProcess => "每进程(10⁰ 量级)",
            Frequency::PerSession => "每会话(10⁰ 量级)",
            Frequency::PerTurn => "每轮(10⁰–10¹ 量级)",
            Frequency::PerBlockChange => "每次块状态变化(10¹ 量级)",
            Frequency::PerInteraction => "每次交互(10⁰–10¹ 量级)",
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub id: &'static str,
    pub kind: PointKind,
    /// What a contribution here supplies.
    pub what: &'static str,
    /// When it is evaluated.
    pub when: &'static str,
    pub frequency: Frequency,
}

/// The twelve points the interface is built out of.
///
/// The number is part of the design (concept §5.6): one more is an argument to
/// be made, not a commit to be pushed. The ninth, `session.action`, was argued:
/// three things the shell drew for one extension (open in the system space, view
/// a tool call there, archive and export) were one kind of thing with nowhere to go. The tenth,
/// `sidebar.tab`, was argued too: the right column is a docking surface per session, and the
/// pages opened in it (a file preview, the usage, a flow's progress, an output stream) are one
/// kind of thing — a tab type that claims an address or is opened by name — which a space's
/// `details.tabs` could not be, being a space's and not a session's. The eleventh, `shortcut`, was
/// argued as well: the shell's keys were three branches of one listener, any package's key would
/// have been another, and the settings could list none of them — a command with a key per platform,
/// where it is in force and what it runs is one kind of thing, and `command` (completions for what is
/// typed) is not it. The twelfth, `conversation.view`: the centre shows a session as its
/// conversation, and a trajectory (the session's calls by turn) is the same session another way —
/// a view of the centre, switched to at its top, which no `space` member or right-column tab
/// could carry: it takes the conversation's place, it does not sit beside it. Every one of them is drawn by a built-in
/// registration **and asked for by the shell**, which is what makes the count
/// honest — a seam nothing uses is not a seam, it is a line in this list.
/// It has happened: five of the seven points there were once went unasked.
pub const POINTS: &[Point] = &[
    Point {
        id: "space",
        kind: PointKind::Register,
        what: "一个 space:左栏入口与条目、右栏页签、中区模式、新会话的默认值",
        when: "界面启动时",
        frequency: Frequency::PerProcess,
    },
    Point {
        id: "home.card",
        kind: PointKind::Render,
        what: "新对话输入框下面的一张卡",
        when: "新对话的空白页打开时",
        frequency: Frequency::PerInteraction,
    },
    Point {
        id: "tool.row",
        kind: PointKind::Render,
        what: "一次工具调用在流里的那一行:图标、标题、摘要",
        when: "该工具块状态变化时",
        frequency: Frequency::PerBlockChange,
    },
    Point {
        id: "details.panel",
        kind: PointKind::Render,
        what: "详情栏里的一页",
        when: "选中项变化时",
        frequency: Frequency::PerInteraction,
    },
    Point {
        id: "flow.block",
        kind: PointKind::Render,
        what: "流里的一种新块",
        when: "该块状态变化时",
        frequency: Frequency::PerBlockChange,
    },
    Point {
        id: "sidebar.group",
        kind: PointKind::Register,
        what: "会话列表的分组与排序",
        when: "会话集合变化时",
        frequency: Frequency::PerInteraction,
    },
    Point {
        id: "command",
        kind: PointKind::Register,
        what: "一条 `/` 命令或 `@` 引用的补全候选（`trigger` 选哪个菜单；带 `submit` 的 `/` 候选收下发出的 `/<名> <参数>`）",
        when: "命令面板打开时",
        frequency: Frequency::PerInteraction,
    },
    Point {
        id: "settings.section",
        kind: PointKind::Render,
        what: "设置面板里的一节",
        when: "设置页打开时",
        frequency: Frequency::PerInteraction,
    },
    Point {
        id: "session.action",
        kind: PointKind::Register,
        what: "对一个会话、它里面一次工具调用或人的一条消息的动作（`at`：标题栏、会话菜单、工具行、工具行的参数、消息的菜单）",
        when: "标题栏、会话菜单、工具行、人的消息画出时",
        frequency: Frequency::PerInteraction,
    },
    Point {
        id: "sidebar.tab",
        kind: PointKind::Register,
        what: "右栏的一种页签类型：按 `patterns` 认领 `harnix-resource://` 地址，或按 `kind` 打开的页；`guide` 进引导页",
        when: "右栏打开一个地址或一种页时",
        frequency: Frequency::PerInteraction,
    },
    Point {
        id: "shortcut",
        kind: PointKind::Register,
        what: "一条有快捷键的命令：按平台的缺省键、生效的地方（全局、输入框、右栏）、要跑的动作；设置的「快捷键」列出它、可改键",
        when: "按下一个键时；设置的「快捷键」打开时",
        frequency: Frequency::PerInteraction,
    },
    Point {
        id: "conversation.view",
        kind: PointKind::Render,
        what: "中间区里对话之外的另一种视图（轨迹……）：有会话打开、有注册方时，中间区顶部出现「对话 | …」的切换；每个会话各记自己的选择",
        when: "有会话打开的中间区画出时，以及切换时",
        frequency: Frequency::PerInteraction,
    },
];

pub fn point(id: &str) -> Option<&'static Point> {
    POINTS.iter().find(|p| p.id == id)
}

/// A point a first-party extension defines for the others to fill, beside the shell's twelve and not
/// counted among them: the extension declares it (`UiDecl::point`) and its UI package defines it.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct ExtensionPoint {
    /// The extension that defines it; the point's name starts with its id.
    pub owner: &'static str,
    pub id: &'static str,
    pub kind: PointKind,
    pub what: &'static str,
    pub when: &'static str,
    pub frequency: Frequency,
}

impl ExtensionPoint {
    /// The declaration its owner registers.
    pub fn decl(&self) -> PointDecl {
        PointDecl::new(self.id, self.kind, self.frequency).what(self.what).when(self.when)
    }
}

/// The points first-party extensions define. Listed here, with the shell's, so the generated
/// catalog shows every point a UI package can fill; each owner declares its own from this list.
pub const EXTENSION_POINTS: &[ExtensionPoint] = &[
    ExtensionPoint {
        owner: "system",
        id: "system.console.tab",
        kind: PointKind::Register,
        what: "系统 space 控制台的一个页签（`{id, title(), render(), order?, about?()}`）",
        when: "系统 space 的控制台画出时",
        frequency: Frequency::PerInteraction,
    },
    ExtensionPoint {
        owner: "system",
        id: "system.inspector.tab",
        kind: PointKind::Render,
        what: "系统 space 右栏里选中会话的一页（`{id, title(target), match(target), render(target), tools?}`）",
        when: "右栏的选中项变化时",
        frequency: Frequency::PerInteraction,
    },
    ExtensionPoint {
        owner: "system",
        id: "system.metrics.link",
        kind: PointKind::Register,
        what: "系统 space 指标条上一格数字去哪（`{id, metric, title(), open()}`）：`metric: \"waiting\"` 的「等你处理」打开待我处理",
        when: "指标条画出时、点那一格时",
        frequency: Frequency::PerInteraction,
    },
    ExtensionPoint {
        owner: "harnix-app",
        id: "harnix-app.project.items",
        kind: PointKind::Register,
        what: "项目树里会话之外的一种条目（`{id, title(), rows(root), subscribe(fn), render(row), open(row), menu?(row), create?, projectMenu?(context)}`）：流程的任务",
        when: "项目 space 的树画出时、条目的 `subscribe` 说变了时",
        frequency: Frequency::PerInteraction,
    },
    ExtensionPoint {
        owner: "harnix-app",
        id: "harnix-app.project.panels",
        kind: PointKind::Render,
        what: "项目概况页的一个区块（`{id, title(), order?, render(context)}`）",
        when: "项目的概况页画出时",
        frequency: Frequency::PerInteraction,
    },
    ExtensionPoint {
        owner: "harnix-app",
        id: "harnix-app.inbox",
        kind: PointKind::Register,
        what: "待我处理的一个来源（`{id, items(), subscribe(fn), open(item)}`）：中枢的提问、流程的确认、hold、外部任务",
        when: "来源的 `subscribe` 说变了时、点一项时",
        frequency: Frequency::PerInteraction,
    },
    ExtensionPoint {
        owner: "harnix-app",
        id: "harnix-app.session.badge",
        kind: PointKind::Render,
        what: "左栏会话行上的一个标记（`{id, render(session)}`）：定时的时钟",
        when: "会话行画出时",
        frequency: Frequency::PerInteraction,
    },
    ExtensionPoint {
        owner: "flow",
        id: "flow.task.badge",
        kind: PointKind::Render,
        what: "任务行上的一个标记（`{id, render(run)}`）：定时的下一次时间",
        when: "任务行画出时",
        frequency: Frequency::PerInteraction,
    },
    ExtensionPoint {
        owner: "flow",
        id: "flow.start.action",
        kind: PointKind::Register,
        what: "新任务对话框和流程详情里「现在启动」之外的另一种起法（`{id, label(), icon?, run(draft)}`）：定时运行",
        when: "新任务对话框、流程详情画出时",
        frequency: Frequency::PerInteraction,
    },
    ExtensionPoint {
        owner: "observability",
        id: "observability.usage.section",
        kind: PointKind::Render,
        what: "系统 space「用量」页签里的一个区块（`{id, title(), order?, render(context)}`，`context` 是 `{since, days}`）：流程按流程的用量",
        when: "「用量」页签画出时",
        frequency: Frequency::PerInteraction,
    },
];

/// The points `owner` defines.
pub fn points_of(owner: &str) -> impl Iterator<Item = &'static ExtensionPoint> + '_ {
    EXTENSION_POINTS.iter().filter(move |p| p.owner == owner)
}

/// The table `docs/contribution_points.md` carries between its generated
/// markers. Changing a point changes the doc; a test enforces it.
pub fn render_markdown() -> String {
    let mut out = String::new();
    out.push_str("| 贡献点 | 类型 | 贡献什么 | 求值时机 | 频率 |\n");
    out.push_str("|---|---|---|---|---|\n");
    for p in POINTS {
        let kind = p.kind.as_str();
        let _ = writeln!(
            out,
            "| `{}` | {} | {} | {} | {} |",
            p.id,
            kind,
            p.what,
            p.when,
            p.frequency.as_str()
        );
    }
    out
}

/// The second table in `docs/contribution_points.md`: the points first-party extensions define.
pub fn render_extension_markdown() -> String {
    let mut out = String::new();
    out.push_str("| 贡献点 | 定义者 | 类型 | 贡献什么 | 求值时机 | 频率 |\n");
    out.push_str("|---|---|---|---|---|---|\n");
    for p in EXTENSION_POINTS {
        let _ = writeln!(
            out,
            "| `{}` | `{}` | {} | {} | {} | {} |",
            p.id,
            p.owner,
            p.kind.as_str(),
            p.what,
            p.when,
            p.frequency.as_str()
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An extension's point is named under its id, and is never one of the shell's.
    #[test]
    fn an_extension_point_is_its_owners() {
        for p in EXTENSION_POINTS {
            assert!(p.id.starts_with(&format!("{}.", p.owner)), "{} is not under {}", p.id, p.owner);
            assert!(point(p.id).is_none(), "{} is one of the shell's", p.id);
        }
        assert_eq!(points_of("system").count(), 3);
        assert_eq!(points_of("harnix-app").count(), 4);
        assert_eq!(points_of("flow").count(), 2);
    }

    /// Twelve, and adding one is a decision (concept §5.6).
    #[test]
    fn the_interface_has_twelve_points() {
        assert_eq!(POINTS.len(), 12);
    }

    /// Nothing here may fire per streaming delta — the whole reason frequency
    /// is stated in the contract (concept §5.6).
    #[test]
    fn no_point_follows_the_stream() {
        for p in POINTS {
            assert!(
                matches!(
                    p.frequency,
                    Frequency::PerProcess
                        | Frequency::PerSession
                        | Frequency::PerBlockChange
                        | Frequency::PerInteraction
                        | Frequency::PerTurn
                ),
                "{} fires too often to be contributable",
                p.id
            );
        }
    }
}
