/**
 * @harnix/flow — the flow module's optional interface (xops-01 §7, flow-design §17).
 *
 * Only `@harnix/react`. The components work wherever `useNest()` does; `registerFlowSpace()` puts
 * them together as one space. The wire shapes are in `types.ts`, in one place.
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
  STATION_STATUSES,
  actorLabel,
  catalogEntries,
  exitKinds,
  formatTime,
  methodName,
  pretty,
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
  workLabel,
  workOf,
  type BlockShape,
  type SchemaRow,
  type StationActions,
  type Tone,
  type WorkSummary,
} from "./model";
export { useMethod, useRunView, useTopic, type Loaded } from "./hooks";
export { FlowGraph, clipText, type FlowGraphProps } from "./FlowGraph";
export { StationPanel, type StationPanelProps } from "./StationPanel";
export { RunList, StartForm, type RunListProps } from "./RunList";
export { FlowReference, type FlowReferenceProps } from "./FlowReference";
export { ProposalCard, type ProposalCardProps } from "./ProposalCard";
export { FlowSpaceState, registerFlowSpace, type FlowSelection, type FlowSpace, type FlowSpaceOptions } from "./space";
export { STRINGS, STRINGS_OWNER, registerFlowStrings } from "./strings";
