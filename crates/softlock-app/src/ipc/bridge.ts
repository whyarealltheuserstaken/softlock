import product from "../strings/product-name.json";
import ui from "../strings/ui.json";

const IPC_BRIDGES = ["fake", "tauri"] as const;

export type IpcBridgeKind = (typeof IPC_BRIDGES)[number];

const FAKE_BRIDGE: IpcBridgeKind = IPC_BRIDGES[0];
const TAURI_BRIDGE: IpcBridgeKind = IPC_BRIDGES[1];

/** Same minimum the service field uses. A shorter PIN is a field error. */
export const MIN_PIN_LEN = 4;

export type LockSourceName = "Hotkey" | "Tray" | "App";

export interface OwnerView {
  serviceRunning: boolean;
  locked: boolean;
  state: string;
  lockSource: string | null;
  sources: string;
  setPinCount: number;
  hardLockCount: number;
  quit: boolean;
  serviceStop: boolean;
  fieldError: string | null;
  notice: string;
}

/** Same shape as the Tauri `platform_status` command. */
export interface PlatformStatus {
  supported: boolean;
  message: string | null;
}

export interface IpcBridge {
  readonly kind: IpcBridgeKind;
  lock(source: LockSourceName): Promise<OwnerView>;
  hardLock(): Promise<OwnerView>;
  setPin(pin: string): Promise<OwnerView>;
  subscribe(onChange: (view: OwnerView) => void): () => void;
  quit(): OwnerView;
  current(): OwnerView;
}

declare global {
  interface Window {
    __SOFTLOCK_PLATFORM_STATUS__?: PlatformStatus;
    __SOFTLOCK_PUSH_STATE__?: (state: string) => void;
    __SOFTLOCK_FAIL_PIPE__?: () => void;
    __SOFTLOCK_PIN_RESULT__?: string;
    __SOFTLOCK_SET_NOTICE__?: (notice: string) => void;
    __SOFTLOCK_FALLBACK__?: (atMs: number, reason: string) => void;
  }
}

export function fallbackNotice(atMs: number, reason: string): string {
  return `${product.productName} switched to the Windows lock at ${atMs} ms because ${reason}`;
}

export function statusLine(view: OwnerView): string {
  const word = view.locked ? ui.statusOn : ui.statusOff;
  return `${product.productName} ${word}`;
}

export function serviceLine(view: OwnerView): string {
  return view.serviceRunning ? ui.serviceRunning : ui.serviceStopped;
}

export function pinErrorText(code: string | null): string | null {
  if (code === "too-short") {
    return ui.pinTooShort;
  }
  if (code === "confirmation-needed") {
    return ui.confirmationNeeded;
  }
  return null;
}

function emptyView(): OwnerView {
  return {
    serviceRunning: true,
    locked: false,
    state: "Unlocked",
    lockSource: null,
    sources: "",
    setPinCount: 0,
    hardLockCount: 0,
    quit: false,
    serviceStop: false,
    fieldError: null,
    notice: "",
  };
}

function failClosed(previous: OwnerView): OwnerView {
  return {
    ...previous,
    serviceRunning: false,
    locked: true,
    state: "Locked",
    fieldError: previous.fieldError,
    serviceStop: false,
  };
}

class FakeBridge implements IpcBridge {
  readonly kind = FAKE_BRIDGE;
  private view = emptyView();
  private listeners = new Set<(view: OwnerView) => void>();

  constructor() {
    window.__SOFTLOCK_PUSH_STATE__ = (state: string) => {
      this.view = {
        ...this.view,
        serviceRunning: true,
        state,
        locked: state !== "Unlocked",
      };
      this.emit();
    };
    window.__SOFTLOCK_FAIL_PIPE__ = () => {
      this.view = failClosed(this.view);
      this.emit();
    };
    window.__SOFTLOCK_SET_NOTICE__ = (notice: string) => {
      this.view = { ...this.view, notice };
      this.emit();
    };
    window.__SOFTLOCK_FALLBACK__ = (atMs: number, reason: string) => {
      this.view = {
        ...this.view,
        serviceRunning: true,
        locked: false,
        state: "Unlocked",
        notice: fallbackNotice(atMs, reason),
      };
      this.emit();
    };
  }

  current(): OwnerView {
    return this.view;
  }

  subscribe(onChange: (view: OwnerView) => void): () => void {
    this.listeners.add(onChange);
    onChange(this.view);
    return () => {
      this.listeners.delete(onChange);
    };
  }

  async lock(source: LockSourceName): Promise<OwnerView> {
    if (!this.view.serviceRunning) {
      this.view = failClosed(this.view);
      this.emit();
      return this.view;
    }
    const sources = this.view.sources ? `${this.view.sources},${source}` : source;
    this.view = {
      ...this.view,
      serviceRunning: true,
      locked: true,
      state: "Locked",
      lockSource: source,
      sources,
      serviceStop: false,
    };
    this.emit();
    return this.view;
  }

  async hardLock(): Promise<OwnerView> {
    if (!this.view.serviceRunning) {
      this.view = failClosed(this.view);
      this.emit();
      return this.view;
    }
    this.view = {
      ...this.view,
      hardLockCount: this.view.hardLockCount + 1,
      serviceStop: false,
    };
    this.emit();
    return this.view;
  }

  async setPin(pin: string): Promise<OwnerView> {
    if ([...pin].length < MIN_PIN_LEN) {
      this.view = { ...this.view, fieldError: "too-short" };
      this.emit();
      return this.view;
    }
    if (window.__SOFTLOCK_PIN_RESULT__ === "needs-confirmation") {
      this.view = { ...this.view, fieldError: "confirmation-needed" };
      this.emit();
      return this.view;
    }
    if (!this.view.serviceRunning) {
      this.view = { ...failClosed(this.view), fieldError: null };
      this.emit();
      return this.view;
    }
    this.view = {
      ...this.view,
      fieldError: null,
      setPinCount: this.view.setPinCount + 1,
      serviceStop: false,
    };
    this.emit();
    return this.view;
  }

  quit(): OwnerView {
    this.view = { ...this.view, quit: true, serviceStop: false };
    this.emit();
    return this.view;
  }

  private emit() {
    for (const listener of this.listeners) {
      listener(this.view);
    }
  }
}

class TauriBridge implements IpcBridge {
  readonly kind = TAURI_BRIDGE;
  private view: OwnerView = failClosed(emptyView());
  private listeners = new Set<(view: OwnerView) => void>();

  current(): OwnerView {
    return this.view;
  }

  subscribe(onChange: (view: OwnerView) => void): () => void {
    this.listeners.add(onChange);
    onChange(this.view);
    void this.refresh();
    void this.listen();
    return () => {
      this.listeners.delete(onChange);
    };
  }

  async lock(source: LockSourceName): Promise<OwnerView> {
    if (source !== "App") {
      return this.view;
    }
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const dto = await invoke<ViewDto>("lock_from_app");
      this.apply(dto);
    } catch {
      this.view = failClosed(this.view);
      this.emit();
    }
    return this.view;
  }

  async hardLock(): Promise<OwnerView> {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const dto = await invoke<ViewDto>("lock_for_real");
      this.apply(dto);
      this.view = { ...this.view, hardLockCount: this.view.hardLockCount + 1 };
    } catch {
      this.view = failClosed(this.view);
      this.emit();
    }
    return this.view;
  }

  async setPin(pin: string): Promise<OwnerView> {
    if ([...pin].length < MIN_PIN_LEN) {
      this.view = { ...this.view, fieldError: "too-short" };
      this.emit();
      return this.view;
    }
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const outcome = await invoke<{ accepted: boolean; field_error: string | null }>("set_pin", {
        pin,
      });
      this.view = {
        ...this.view,
        fieldError: outcome.field_error,
        setPinCount: outcome.accepted ? this.view.setPinCount + 1 : this.view.setPinCount,
      };
      if (!outcome.accepted && outcome.field_error === null) {
        this.view = failClosed(this.view);
      }
    } catch {
      this.view = failClosed({ ...this.view, fieldError: null });
    }
    this.emit();
    return this.view;
  }

  quit(): OwnerView {
    this.view = { ...this.view, quit: true, serviceStop: false };
    this.emit();
    void import("@tauri-apps/api/core").then(({ invoke }) => invoke("quit_app"));
    return this.view;
  }

  private async refresh() {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const dto = await invoke<ViewDto>("owner_view");
      this.apply(dto);
    } catch {
      this.view = failClosed(this.view);
      this.emit();
    }
  }

  private async listen() {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      await listen<ViewDto>("owner-view", (event) => {
        this.apply(event.payload);
      });
    } catch {
      this.view = failClosed(this.view);
      this.emit();
    }
  }

  private apply(dto: ViewDto) {
    const source = dto.lock_source;
    const sources =
      source && !this.view.sources.split(",").includes(source)
        ? this.view.sources
          ? `${this.view.sources},${source}`
          : source
        : this.view.sources;
    this.view = {
      ...this.view,
      serviceRunning: dto.service_running,
      locked: dto.locked || !dto.service_running,
      state: dto.service_running ? dto.state : "Locked",
      lockSource: source,
      sources,
      serviceStop: false,
      notice: dto.notice ?? this.view.notice,
    };
    this.emit();
  }

  private emit() {
    for (const listener of this.listeners) {
      listener(this.view);
    }
  }
}

interface ViewDto {
  service_running: boolean;
  locked: boolean;
  state: string;
  lock_source: string | null;
  status_line: string;
  service_line: string;
  notice?: string;
}

export function createIpcBridge(): IpcBridge {
  const kind =
    import.meta.env.VITE_IPC_BRIDGE === FAKE_BRIDGE ? FAKE_BRIDGE : TAURI_BRIDGE;
  return kind === FAKE_BRIDGE ? new FakeBridge() : new TauriBridge();
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
