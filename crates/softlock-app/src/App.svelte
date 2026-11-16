<script lang="ts">
  import { createIpcBridge } from "./ipc/bridge";
  import product from "./strings/product-name.json" with { type: "json" };

  const bridge = createIpcBridge();
  document.title = product.productName;
</script>

<main data-ipc-bridge={bridge.kind}></main>
