<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { Droplets, FolderOpen, Trash2 } from "@lucide/svelte";
  import type { FileReport, CleanResult, CleanMode } from "$lib/types";
  import { inspectFiles, cleanFiles, pickFiles, isAccepted } from "$lib/laver";
  import FileCard from "$lib/FileCard.svelte";

  let reports = $state<FileReport[]>([]);
  let results = $state<Record<string, CleanResult>>({});
  let progress = $state<Record<string, number>>({});
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
  const hasVideo = $derived(reports.some((r) => r.isVideo));

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
    if (cleanable.length === 0 || cleaning) return;
    cleaning = true;
    progress = {};
    try {
      const done = await cleanFiles(
        cleanable.map((r) => r.path),
        { mode, renameNeutral },
        (ev) => (progress = { ...progress, [ev.src]: ev.percent }),
      );
      const next = { ...results };
      for (const r of done) next[r.src] = r;
      results = next;
    } finally {
      cleaning = false;
      progress = {};
    }
  }

  function clearAll() {
    reports = [];
    results = {};
    progress = {};
  }

  function onKey(e: KeyboardEvent) {
    const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "o") {
      e.preventDefault();
      browse();
    } else if (e.key === "Escape" && !typing && reports.length > 0 && !cleaning) {
      clearAll();
    }
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
    window.addEventListener("keydown", onKey);
  });
  onDestroy(() => {
    unlisten?.();
    window.removeEventListener("keydown", onKey);
  });
</script>

{#if reports.length === 0}
  <section class="flex h-full flex-col items-center justify-center gap-9 p-10">
    <button
      onclick={browse}
      class="group flex w-full max-w-md flex-col items-center rounded-xl border border-dashed px-10 py-14 text-center transition-colors
        {dragging ? 'border-accent bg-accent/[0.04]' : 'border-edge hover:border-dim'}"
    >
      <Droplets
        size={30}
        strokeWidth={1.5}
        class="mb-5 transition-colors {dragging ? 'text-accent' : 'text-faint group-hover:text-dim'}"
      />
      <p class="text-[15px]">Dépose des photos ou des vidéos</p>
      <p class="mt-2 max-w-xs text-[13px] leading-relaxed text-dim">
        Les métadonnées inspectées, puis retirées sur place. L'original n'est jamais touché.
      </p>
      <span class="mt-6 flex items-center gap-1.5 text-[12px] text-faint">
        clique, ou
        <kbd class="inline-flex items-center rounded border border-edge bg-raised px-1 py-px font-mono text-[11px] leading-none">Ctrl</kbd>
        <kbd class="inline-flex items-center rounded border border-edge bg-raised px-1 py-px font-mono text-[11px] leading-none">O</kbd>
      </span>
    </button>
    <p class="text-[12px] tracking-wide text-faint">Tout ressort propre. Rien ne reste.</p>
  </section>
{:else}
  <div class="relative flex h-full flex-col">
    <!-- Barre d'action -->
    <div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-edge px-5 py-2.5">
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

      {#if hasVideo}
        <span class="text-[11px] text-faint" title="Le lavage vidéo est un remux complet, sans réencodage.">
          Vidéos : nettoyage complet
        </span>
      {/if}

      <label class="flex cursor-pointer items-center gap-2 text-[12px] text-dim">
        <input type="checkbox" bind:checked={renameNeutral} class="accent-accent" />
        Renommer en neutre
      </label>

      <div class="grow"></div>

      <span class="text-[12px] text-faint">
        {reports.length} fichier{reports.length > 1 ? "s" : ""}
        {#if cleanable.length !== reports.length}· {cleanable.length} lavable{cleanable.length > 1 ? "s" : ""}{/if}
      </span>
      <button onclick={browse} class="flex items-center gap-1.5 text-[12px] text-dim transition-colors hover:text-text">
        <FolderOpen size={13} />Ajouter
      </button>
      <button onclick={clearAll} class="flex items-center gap-1.5 text-[12px] text-dim transition-colors hover:text-text">
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
    <div class="min-h-0 flex-1 space-y-2.5 overflow-y-auto p-5">
      {#if inspecting}
        <p class="text-[13px] text-dim">Inspection…</p>
      {/if}
      {#each reports as report (report.path)}
        <FileCard
          {report}
          result={results[report.path] ?? null}
          busy={cleaning && !results[report.path] && report.canClean && !report.error}
          progress={progress[report.path] ?? null}
        />
      {/each}
    </div>

    <!-- Overlay de dépôt -->
    {#if dragging}
      <div class="pointer-events-none absolute inset-0 grid place-items-center bg-bg/80 backdrop-blur-sm">
        <div class="rounded-xl border-2 border-dashed border-accent px-10 py-8 text-[15px] text-accent">
          Dépose pour ajouter
        </div>
      </div>
    {/if}
  </div>
{/if}
