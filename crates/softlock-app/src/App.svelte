<script lang="ts">
  import { createIpcBridge } from "./ipc/bridge";

  const bridge = createIpcBridge();
</script>

<main data-ipc-bridge={bridge.kind}></main>
