<script lang="ts">
  import { Download, FolderOpen, X, FileCheck2, TriangleAlert, Music } from "@lucide/svelte";
  import { cancelDownload, formatBytes, formatEta } from "./recuperer";
  import { openFile, revealFolder } from "./laver";

  export type Job = {
    key: string;
    id: string | null;
    label: string;
    source: string | null;
    audio: boolean;
    status: "queued" | "downloading" | "postprocess" | "washing" | "done" | "error" | "cancelled";
    downloaded: number | null;
    total: number | null;
    speed: number | null;
    eta: number | null;
    stage: string | null;
    path: string | null;
    name: string | null;
    error: string | null;
  };

  let { job }: { job: Job } = $props();

  let openErr = $state<string | null>(null);

  async function reveal(fn: (p: string) => Promise<void>) {
    if (!job.path) return;
    openErr = null;
    try {
      await fn(job.path);
    } catch (e) {
      openErr = String(e);
    }
  }

  const percent = $derived(
    job.total && job.downloaded !== null ? Math.min(100, (job.downloaded / job.total) * 100) : null,
  );
  const active = $derived(
    job.status === "queued" || job.status === "downloading" || job.status === "postprocess",
  );
  const running = $derived(job.status !== "done" && job.status !== "error" && job.status !== "cancelled");
</script>

<article class="rounded-lg border border-edge bg-surface px-4 py-3">
  <div class="flex items-center gap-3">
    <div class="grid size-9 shrink-0 place-items-center rounded-md border border-edge bg-raised text-dim">
      {#if job.status === "done"}
        <FileCheck2 size={16} class="text-accent" />
      {:else if job.status === "error"}
        <TriangleAlert size={16} class="text-danger" />
      {:else if job.audio}
        <Music size={15} />
      {:else}
        <Download size={15} />
      {/if}
    </div>

    <div class="min-w-0 flex-1">
      <div class="flex items-baseline gap-2">
        <span class="truncate text-[13px] font-medium" title={job.label}>{job.label}</span>
        {#if job.source}
          <span class="shrink-0 text-[11px] text-dim">{job.source}</span>
        {/if}
      </div>

      <p class="mt-0.5 truncate text-[12px] text-dim">
        {#if job.status === "queued"}
          En attente…
        {:else if job.status === "downloading"}
          {formatBytes(job.downloaded)}{#if job.total} / {formatBytes(job.total)}{/if}
          {#if job.speed}· {formatBytes(job.speed)}/s{/if}
          {#if formatEta(job.eta)}· {formatEta(job.eta)} restant{/if}
        {:else if job.status === "postprocess"}
          {job.stage ?? "Finalisation"}…
        {:else if job.status === "washing"}
          Lavage des métadonnées…
        {:else if job.status === "done"}
          <span class="font-mono text-[12px]">{job.name}</span>
        {:else if job.status === "cancelled"}
          Annulé
        {:else if job.status === "error"}
          <span class="text-danger">{job.error}</span>
        {/if}
      </p>
      {#if openErr}
        <p class="mt-0.5 truncate text-[11px] text-danger" title={openErr}>Ouverture impossible : {openErr}</p>
      {/if}
    </div>

    <div class="flex shrink-0 items-center gap-2">
      {#if job.status === "done"}
        <button onclick={() => reveal(openFile)} class="text-[12px] text-dim hover:text-text">Ouvrir</button>
        <button onclick={() => reveal(revealFolder)} class="flex items-center gap-1 text-[12px] text-dim hover:text-text">
          <FolderOpen size={12} />Dossier
        </button>
      {:else if active && job.id}
        <button
          onclick={() => job.id && cancelDownload(job.id)}
          title="Annuler"
          class="flex items-center gap-1 rounded px-1.5 py-1 text-[12px] text-dim hover:text-danger"
        >
          <X size={13} />
        </button>
      {/if}
    </div>
  </div>

  {#if running}
    <div class="mt-2.5 h-1 overflow-hidden rounded-full bg-raised">
      {#if percent !== null}
        <div class="h-full rounded-full bg-accent transition-[width] duration-300" style="width: {percent}%"></div>
      {:else}
        <!-- Taille inconnue (flux, live) : barre indéterminée -->
        <div class="indeterminate h-full w-1/3 rounded-full bg-accent/70"></div>
      {/if}
    </div>
  {/if}
</article>
