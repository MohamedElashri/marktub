<script lang="ts">
  let {
    value,
    min,
    max,
    reset,
    onpreview,
    oncommit,
  }: {
    value: number;
    min: number;
    max: number;
    reset: number;
    onpreview: (value: number | null) => void;
    oncommit: (value: number) => void;
  } = $props();
  let drag: {
    pointer: number;
    x: number;
    width: number;
    value: number;
  } | null = null;
  const bound = (width: number) =>
    Math.round(Math.max(min, Math.min(max, width)));
  function finish(event: PointerEvent, cancel = false) {
    if (!drag || drag.pointer !== event.pointerId) return;
    const width = drag.value;
    drag = null;
    if (cancel) onpreview(null);
    else oncommit(width);
  }
</script>

<svelte:window
  onpointermove={(event) => {
    if (!drag || event.pointerId !== drag.pointer) return;
    drag.value = bound(drag.width + event.clientX - drag.x);
    onpreview(drag.value);
  }}
  onpointerup={(event) => finish(event)}
  onpointercancel={(event) => finish(event, true)}
/>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions (ARIA window splitters are focusable separators.) -->
<div
  class="library-divider"
  role="separator"
  tabindex="0"
  aria-label="Resize library"
  aria-orientation="vertical"
  aria-controls="library-sidebar"
  aria-valuemin={min}
  aria-valuemax={max}
  aria-valuenow={value}
  aria-valuetext={`${value} pixels`}
  title="Drag to resize. Use arrow keys, or double-click to reset."
  onpointerdown={(event) => {
    if (event.button !== 0 || drag) return;
    event.preventDefault();
    event.currentTarget.focus();
    event.currentTarget.setPointerCapture(event.pointerId);
    drag = { pointer: event.pointerId, x: event.clientX, width: value, value };
  }}
  onlostpointercapture={(event) => finish(event, true)}
  ondblclick={() => oncommit(bound(reset))}
  onkeydown={(event) => {
    if (event.key === "Escape" && drag) {
      drag = null;
      onpreview(null);
      return;
    }
    const step = event.shiftKey ? 48 : 16;
    const next =
      event.key === "ArrowLeft"
        ? value - step
        : event.key === "ArrowRight"
          ? value + step
          : event.key === "Home"
            ? min
            : event.key === "End"
              ? max
              : event.key === "Enter"
                ? reset
                : null;
    if (next !== null) {
      event.preventDefault();
      oncommit(bound(next));
    }
  }}
></div>
