interface ImportMetaEnv {
  readonly VITE_IPC_BRIDGE?: "fake" | "tauri";
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
