<script lang="ts">
  import { tick } from "svelte";
  import { binding, shortcutLabel, score, type Command } from "./commands";
  import type { Values } from "./preferences";
  let {
    commands,
    values,
    onrun,
  }: {
    commands: Command[];
    values: Values;
    onrun: (command: Command) => void;
  } = $props();
  let dialog: HTMLDialogElement, input: HTMLInputElement;
  let query = $state(""),
    at = $state(0),
    recent = $state<string[]>([]);
  let results = $derived(
    commands
      .map((command) => ({
        command,
        score: query.trim()
          ? score(`${command.label} ${command.group}`, query)
          : recent.includes(command.id)
            ? recent.indexOf(command.id) - 20
            : 0,
      }))
      .filter((r) => Number.isFinite(r.score))
      .sort((a, b) => a.score - b.score)
      .slice(0, 30),
  );
  export async function show() {
    query = "";
    at = 0;
    dialog.showModal();
    await tick();
    input.focus();
  }
  function run(command: Command | undefined) {
    if (command?.enabled) {
      recent = [command.id, ...recent.filter((id) => id !== command.id)].slice(
        0,
        10,
      );
      dialog.close();
      onrun(command);
    }
  }
</script>

<dialog bind:this={dialog} class="command-palette" aria-label="Command palette">
  <div class="palette-search">
    <input
      bind:this={input}
      aria-label="Search commands"
      bind:value={query}
      oninput={() => (at = 0)}
      placeholder="Find an action…"
      role="combobox"
      aria-autocomplete="list"
      aria-expanded="true"
      aria-controls="command-results"
      aria-activedescendant={results[at]
        ? `command-${results[at].command.id}`
        : undefined}
      onkeydown={(event) => {
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
          event.preventDefault();
          at =
            (at + (event.key === "ArrowDown" ? 1 : -1) + results.length) %
            Math.max(results.length, 1);
          document
            .getElementById(`command-${results[at]?.command.id}`)
            ?.scrollIntoView({ block: "nearest" });
        }
        if (event.key === "Enter") {
          event.preventDefault();
          run(results[at]?.command);
        }
      }}
    /><button onclick={() => dialog.close()}>Esc</button>
  </div>
  <div id="command-results" role="listbox" aria-label="Commands">
    {#each results as { command }, i (command.id)}<button
        id={`command-${command.id}`}
        role="option"
        tabindex="-1"
        aria-selected={i === at}
        aria-disabled={!command.enabled}
        onclick={() => run(command)}
        onpointermove={() => (at = i)}
        ><span
          >{command.label}<small
            >{command.enabled
              ? recent.includes(command.id)
                ? `Recent · ${command.group}`
                : command.group
              : (command.reason ?? "Unavailable in this context")}</small
          ></span
        >{#if shortcutLabel(binding(command, values)).visual}<kbd
            aria-label={shortcutLabel(binding(command, values)).spoken}
            >{shortcutLabel(binding(command, values)).visual}</kbd
          >{/if}</button
      >{/each}{#if !results.length}<p>No matching action.</p>{/if}
  </div>
</dialog>
