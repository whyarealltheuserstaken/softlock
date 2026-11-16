const IPC_BRIDGES = ["fake", "tauri"] as const;

export type IpcBridgeKind = (typeof IPC_BRIDGES)[number];

const FAKE_BRIDGE: IpcBridgeKind = IPC_BRIDGES[0];
const TAURI_BRIDGE: IpcBridgeKind = IPC_BRIDGES[1];

/** Stand-in for the service pipe. No commands exist yet. */
export interface IpcBridge {
  readonly kind: IpcBridgeKind;
}

/** Same shape as the Tauri `platform_status` command. */
export interface PlatformStatus {
  supported: boolean;
  message: string | null;
}

declare global {
  interface Window {
    __SOFTLOCK_PLATFORM_STATUS__?: PlatformStatus;
  }
}

export function createIpcBridge(): IpcBridge {
  const kind =
    import.meta.env.VITE_IPC_BRIDGE === FAKE_BRIDGE ? FAKE_BRIDGE : TAURI_BRIDGE;
  return { kind };
}

/** Reads the host status. The fake bridge uses an injected value; Tauri calls the command. */
export async function loadPlatformStatus(kind: IpcBridgeKind): Promise<PlatformStatus> {
  if (kind === TAURI_BRIDGE) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke<PlatformStatus>("platform_status");
  }
  const injected = window.__SOFTLOCK_PLATFORM_STATUS__;
  if (injected && typeof injected.supported === "boolean") {
    return {
      supported: injected.supported,
      message: injected.message ?? null,
    };
  }
  return { supported: true, message: null };
}
