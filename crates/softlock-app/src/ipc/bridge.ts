export type IpcBridgeKind = "fake" | "tauri";

/** Stand-in for the service pipe. No commands exist yet. */
export interface IpcBridge {
  readonly kind: IpcBridgeKind;
}

export function createIpcBridge(): IpcBridge {
  const kind = import.meta.env.VITE_IPC_BRIDGE === "fake" ? "fake" : "tauri";
  return { kind };
}
