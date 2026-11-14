<script lang="ts">
  let { active, label = "Working…" }: { active: boolean; label?: string } =
    $props();
  let visible = $state(false);
  $effect(() => {
    visible = false;
    if (!active) return;
    const timer = setTimeout(() => (visible = true), 180);
    return () => clearTimeout(timer);
  });
</script>

{#if visible}<div
    class="operation-status"
    role="status"
    aria-live="polite"
    aria-atomic="true"
  >
    <span class="operation-spinner" aria-hidden="true"></span><span
      >{label}</span
    >
  </div>{/if}
