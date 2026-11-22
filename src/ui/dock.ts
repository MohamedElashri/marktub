// Keep one live control tree when its toolbar lives outside the owning pane.
// Both locations stay under the same Svelte event root.
export function dock(node: HTMLElement, target: HTMLElement | undefined) {
  const home = document.createComment("toolbar home");
  node.before(home);
  function place(next: HTMLElement | undefined) {
    if (next) next.append(node);
    else home.parentNode?.insertBefore(node, home.nextSibling);
  }
  place(target);
  return {
    update: place,
    destroy() {
      node.remove();
      home.remove();
    },
  };
}
