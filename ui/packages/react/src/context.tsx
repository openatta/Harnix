/**
 * React 侧的接缝：一个客户端、一张文案表，经 context 交给组件；连接状态与会话状态经
 * `useSyncExternalStore` 订阅。
 */

import { createContext, useCallback, useContext, useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import type { ConnectionStatus, Hello, HarnixClient } from "@harnix/core";
import { SessionController, SYSTEM_INSTANCE } from "@harnix/core/state";
import type { SessionState } from "@harnix/core/state";
import { zhCN, type HarnixUiStrings } from "./strings";

interface HarnixUiContextValue {
  client: HarnixClient;
  strings: HarnixUiStrings;
  promptTimeoutSecs?: number;
}

const HarnixUiContext = createContext<HarnixUiContextValue | null>(null);

export interface HarnixUiProviderProps {
  client: HarnixClient;
  /** 整张换掉，或只覆盖几项。缺省简体中文。 */
  strings?: Partial<HarnixUiStrings>;
  /** 引擎对未答权限提问的超时（秒），用于卡片上的倒计时。 */
  promptTimeoutSecs?: number;
  children: ReactNode;
}

export function HarnixUiProvider({ client, strings, promptTimeoutSecs, children }: HarnixUiProviderProps) {
  const value = useMemo(
    () => ({ client, strings: { ...zhCN, ...strings }, promptTimeoutSecs }),
    [client, strings, promptTimeoutSecs],
  );
  return <HarnixUiContext.Provider value={value}>{children}</HarnixUiContext.Provider>;
}

function useHarnixUi(): HarnixUiContextValue {
  const value = useContext(HarnixUiContext);
  if (!value) throw new Error("@harnix/react: 组件必须放在 <HarnixUiProvider> 里面");
  return value;
}

export function useHarnixClient(): HarnixClient {
  return useHarnixUi().client;
}

export function useStrings(): HarnixUiStrings {
  return useHarnixUi().strings;
}

/** 连接状态，随客户端的 onStatus 变化。 */
export function useConnection(): { status: ConnectionStatus; detail: string | undefined } {
  const client = useHarnixClient();
  const subscribe = useCallback((fn: () => void) => client.onStatus(fn), [client]);
  const status = useSyncExternalStore(subscribe, () => client.status);
  const detail = useSyncExternalStore(subscribe, () => client.detail);
  return { status, detail };
}

/**
 * 打开一个会话：建控制器、attach、追赶；会话 id 变了或组件卸载时 detach。
 * 返回不可变的会话快照与控制器（发送、中断、回答……）。
 */
export function useSession(
  sessionId: string,
  instance: string = SYSTEM_INSTANCE,
  space?: string,
  extras?: (text: string) => Record<string, unknown>,
): { state: SessionState; controller: SessionController } {
  const { client, strings, promptTimeoutSecs } = useHarnixUi();
  const controller = useMemo(
    () => new SessionController({ client, instance, space, sessionId, strings, promptTimeoutSecs, extras }),
    [client, instance, space, sessionId, strings, promptTimeoutSecs, extras],
  );
  useEffect(() => {
    controller.start();
    return () => controller.stop();
  }, [controller]);
  const state = useSyncExternalStore(controller.subscribe, controller.getSnapshot);
  return { state, controller };
}

/** 每个客户端只问一次 `harnix.hello`（场景、命令、上限）；失败就忘掉，下次连上再问。 */
const helloCache = new WeakMap<HarnixClient, Promise<Hello>>();

/** `harnix.hello` 的应答：连上之后取一次，所有组件共用。还没有（或取不到）是 null。 */
export function useHello(): Hello | null {
  const client = useHarnixClient();
  const { status } = useConnection();
  const [hello, setHello] = useState<Hello | null>(null);
  useEffect(() => {
    if (status !== "open" && status !== "degraded") return;
    let alive = true;
    let pending = helloCache.get(client);
    if (!pending) {
      pending = client.call("harnix.hello", {} as Record<string, never>);
      helloCache.set(client, pending);
    }
    pending.then(
      (h) => alive && setHello(h && typeof h === "object" ? h : null),
      () => {
        helloCache.delete(client);
      },
    );
    return () => {
      alive = false;
    };
  }, [client, status]);
  return hello;
}
