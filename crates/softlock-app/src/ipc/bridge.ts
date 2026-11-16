const IPC_BRIDGES = ["fake", "tauri"] as const;

export type IpcBridgeKind = (typeof IPC_BRIDGES)[number];

const FAKE_BRIDGE: IpcBridgeKind = IPC_BRIDGES[0];
const TAURI_BRIDGE: IpcBridgeKind = IPC_BRIDGES[1];

/** Stand-in for the service pipe. No commands exist yet. */
export interface IpcBridge {
  readonly kind: IpcBridgeKind;
}

export function createIpcBridge(): IpcBridge {
  const kind =
    import.meta.env.VITE_IPC_BRIDGE === FAKE_BRIDGE ? FAKE_BRIDGE : TAURI_BRIDGE;
  return { kind };
}
