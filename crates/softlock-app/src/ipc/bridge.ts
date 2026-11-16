const IPC_BRIDGES = ["fake", "tauri"] as const;

export type IpcBridgeKind = (typeof IPC_BRIDGES)[number];

const FAKE_BRIDGE: IpcBridgeKind = IPC_BRIDGES[0];
const TAURI_BRIDGE: IpcBridgeKind = IPC_BRIDGES[1];

/** Stand-in for the service pipe. No commands exist yet. */
export interface IpcBridge {
  readonly kind: IpcBridgeKind;
  readonly supported: boolean;
}

/** v1 runs on Windows. macOS and Linux stay unsupported until their phases. */
export function platformIsSupported(userAgent: string): boolean {
  return /Windows/i.test(userAgent);
}

export function createIpcBridge(userAgent = navigator.userAgent): IpcBridge {
  const kind =
    import.meta.env.VITE_IPC_BRIDGE === FAKE_BRIDGE ? FAKE_BRIDGE : TAURI_BRIDGE;
  return { kind, supported: platformIsSupported(userAgent) };
}
