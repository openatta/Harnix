/**
 * @harnix/flow — the flow module's interface (xops-01 §7, xops-02 §7, xops-03 §6).
 *
 * Only `@harnix/react`. The components work wherever `useHost()` does; `registerFlowSpace()` puts
 * them together as one space, `registerSystemFlows()` inside another space's console instead.
 * `register` is what the shell activates while the backend's `flow` extension is enabled: a tab of
 * `system.console.tab` where that point is defined, else the flow space of its own. The wire
 * shapes are in `types.ts`, in one place.
 */

import "./flow.css";

export * from "./types";
export {
  backEdgePath,
  backEdges,
  backLanes,
  countCrossings,
  edgePath,
  forwardEdges,
  layersOf,
  layoutFlow,
  needsOf,
  type BackEdge,
  type LaidEdge,
  type LaidNode,
  type Layout,
  type LayoutOptions,
  type LayoutStation,
} from "./layout";
export {
  MAIN_SESSION_KEY,
  PHASE_STATES,
  RUN_TOOL_TARGETS,
  STATION_STATUSES,
  addUsage,
  formatDuration,
  formatTokens,
  guideBlocks,
  inlineOf,
  mainSessionOf,
  phaseOf,
  phaseTone,
  runElapsed,
  runUsage,
  stationElapsed,
  stationUsage,
  tokensOf,
  type GuideBlock,
  type Inline,
  actorLabel,
  catalogEntries,
  checkOk,
  defOf,
  defsOf,
  defsTopic,
  digestLines,
  dryRunsOf,
  dryRunsTopic,
  focusTopic,
  defBlockedKey,
  ORIGINS,
  isDryRun,
  problemText,
  toolTarget,
  exitKinds,
  formatTime,
  methodName,
  openOf,
  pretty,
  leaseUntil,
  proposalHandledOf,
  proposalOf,
  reworkTargets,
  runIdOf,
  runTone,
  runTopic,
  runsOf,
  runsTopic,
  schemaRows,
  sessionOf,
  stationActions,
  stationStyle,
  stationTitle,
  taskProgress,
  taskTone,
  runLive,
  runningIn,
  sameRoot,
  workLabel,
  workOf,
  PART_KINDS,
  gateTitle,
  partDir,
  partTitle,
  permissionScope,
  permissionsOf,
  rulesOf,
  GUIDE_GROUPS,
  guideGroupOf,
  guideSections,
  guideTermOf,
  type GuideGroup,
  type BlockShape,
  type SchemaRow,
  type StationActions,
  type Tone,
  type ToolTarget,
  type WorkSummary,
} from "./model";
export { useDefs, useDryRuns, useMethod, useRunView, useTopic, type Loaded } from "./hooks";
export { FlowGraph, clipText, type FlowGraphProps } from "./FlowGraph";
export { FlowFramework, type FlowFrameworkProps, type FrameworkPick } from "./FlowFramework";
export { StationPanel, stationPermissions, type StationPanelProps } from "./StationPanel";
export { FlowHooks, FlowParts, PermissionList, RuleChips } from "./parts";
export { RunList, StartForm, type RunListProps } from "./RunList";
export { FlowReference, type FlowReferenceProps } from "./FlowReference";
export { ProposalCard, type ProposalCardProps } from "./ProposalCard";
export { FlowToolRow, ToolTargetLine } from "./ToolTarget";
export { extension, register } from "./register";
export { registerFlowSpace, registerSystemFlows, type FlowSpace, type FlowSpaceOptions, type SystemFlowsOptions, type SystemFlowsView } from "./space";
export { FlowContext, SystemFlows } from "./context";
export { FlowSpaceState, type FlowDetailOf, type FlowSelection, type StateTabs } from "./state";
export { revealInGuide } from "./tabs";
export { DryControls, EmptyConversation, FlowDetail, FlowSecondary, FlowsTab, FocusNotice, GuideView, PhaseBadge, ReferenceTab, RunDetail, RunsTab, useSelection, type FlowTabs } from "./tabs";
export { OutputDirLine, RunHead, RunPanel, RunSessionsList, RunStations, WorktreeLine, currentStation, outputDirOf, taskStep, worktreeOf, worktreeOpen, type RunPanelProps } from "./RunPanel";
export { PROJECT_ITEMS, ProjectTasks, TASK_KIND, TASK_TAB_KIND, TaskRow, activityOf as taskActivityOf, awaitMain, openTask, registerTaskItems, taskRows, type ItemContext, type ItemRow } from "./task";
export { collectInputs, defOptions, defaultOutputDir, fieldsOf, initialText, newTaskDialog, noWorktreeText, workspaceParam, type FieldKind, type NewTaskOptions, type SchemaField } from "./NewTask";
export { TaskTab, findRunOf, rememberRun, rememberedRun } from "./TaskTab";
export { STRINGS, STRINGS_OWNER, registerFlowStrings } from "./strings";
