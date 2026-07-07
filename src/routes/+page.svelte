<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { Droplets, FolderOpen, Trash2 } from "@lucide/svelte";
  import type { FileReport, CleanResult, CleanMode } from "$lib/types";
  import { inspectFiles, cleanFiles, pickFiles, isAccepted } from "$lib/laver";
  import FileCard from "$lib/FileCard.svelte";

  let reports = $state<FileReport[]>([]);
  let results = $state<Record<string, CleanResult>>({});
  let dragging = $state(false);
  let inspecting = $state(false);
  let cleaning = $state(false);

  let mode = $state<CleanMode>("all");
  let renameNeutral = $state(false);

  const modes: { id: CleanMode; label: string }[] = [
    { id: "all", label: "Tout retirer" },
    { id: "gps", label: "GPS seulement" },
    { id: "keepDate", label: "Garder la date" },
  ];

  const cleanable = $derived(reports.filter((r) => r.canClean && !r.error));
  const known = $derived(new Set(reports.map((r) => r.path)));

  async function addPaths(paths: string[]) {
    const fresh = paths.filter((p) => isAccepted(p) && !known.has(p));
    if (fresh.length === 0) return;
    inspecting = true;
    try {
      const added = await inspectFiles(fresh);
      reports = [...reports, ...added];
    } finally {
      inspecting = false;
    }
  }

  async function browse() {
    await addPaths(await pickFiles());
  }

  async function washAll() {
    if (cleanable.length === 0) return;
    cleaning = true;
    try {
      const done = await cleanFiles(
        cleanable.map((r) => r.path),
        { mode, renameNeutral },
      );
      const next = { ...results };
      for (const r of done) next[r.src] = r;
      results = next;
    } finally {
      cleaning = false;
    }
  }

  function clearAll() {
    reports = [];
    results = {};
  }

  let unlisten: (() => void) | undefined;
  onMount(async () => {
    unlisten = await getCurrentWebview().onDragDropEvent((event) => {
      const p = event.payload;
      if (p.type === "drop") {
        dragging = false;
        addPaths(p.paths);
      } else if (p.type === "enter" || p.type === "over") {
        dragging = true;
      } else {
        dragging = false;
      }
    });
  });
  onDestroy(() => unlisten?.());
</script>

{#if reports.length === 0}
  <section class="flex h-full items-center justify-center p-10">
    <button
      onclick={browse}
      class="w-full max-w-xl rounded-lg border border-dashed px-10 py-16 text-center transition-colors
        {dragging ? 'border-accent bg-accent/5' : 'border-edge hover:border-dim'}"
    >
      <Droplets size={28} strokeWidth={1.5} class="mx-auto mb-4 {dragging ? 'text-accent' : 'text-dim'}" />
      <p class="text-[15px]">Dépose des photos, ou clique pour choisir</p>
      <p class="mt-2 text-[13px] leading-relaxed text-dim">
        Inspection puis lavage des métadonnées, sur place.<br />
        Les originaux ne sont jamais modifiés.
      </p>
    </button>
  </section>
{:else}
  <div class="relative flex h-full flex-col">
    <!-- Barre d'action -->
    <div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-edge px-5 py-3">
      <div class="flex rounded-md border border-edge p-0.5">
        {#each modes as m}
          <button
            onclick={() => (mode = m.id)}
            class="rounded px-2.5 py-1 text-[12px] transition-colors
              {mode === m.id ? 'bg-raised text-text' : 'text-dim hover:text-text'}"
          >
            {m.label}
          </button>
        {/each}
      </div>

      <label class="flex cursor-pointer items-center gap-2 text-[12px] text-dim">
        <input type="checkbox" bind:checked={renameNeutral} class="accent-accent" />
        Renommer neutrement
      </label>

      <div class="grow"></div>

      <span class="text-[12px] text-dim">
        {reports.length} fichier{reports.length > 1 ? "s" : ""}
        {#if cleanable.length !== reports.length}· {cleanable.length} lavable{cleanable.length > 1 ? "s" : ""}{/if}
      </span>
      <button onclick={browse} class="flex items-center gap-1.5 text-[12px] text-dim hover:text-text">
        <FolderOpen size={13} />Ajouter
      </button>
      <button onclick={clearAll} class="flex items-center gap-1.5 text-[12px] text-dim hover:text-text">
        <Trash2 size={13} />Vider
      </button>
      <button
        onclick={washAll}
        disabled={cleaning || cleanable.length === 0}
        class="rounded-md bg-accent px-3.5 py-1.5 text-[13px] font-medium text-bg transition-opacity
          hover:opacity-90 disabled:opacity-40"
      >
        {cleaning ? "Lavage…" : `Laver ${cleanable.length} fichier${cleanable.length > 1 ? "s" : ""}`}
      </button>
    </div>

    <!-- Liste -->
    <div class="min-h-0 flex-1 space-y-3 overflow-y-auto p-5">
      {#if inspecting}
        <p class="text-[13px] text-dim">Inspection…</p>
      {/if}
      {#each reports as report (report.path)}
        <FileCard {report} result={results[report.path] ?? null} busy={cleaning && !results[report.path] && report.canClean && !report.error} />
      {/each}
    </div>

    <!-- Overlay de dépôt -->
    {#if dragging}
      <div class="pointer-events-none absolute inset-0 grid place-items-center bg-bg/80 backdrop-blur-sm">
        <div class="rounded-lg border-2 border-dashed border-accent px-10 py-8 text-[15px] text-accent">
          Dépose pour ajouter
        </div>
      </div>
    {/if}
  </div>
{/if}
