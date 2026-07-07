<script lang="ts">
  import { MapPin, Copy, Check, ExternalLink, Image, ChevronDown, FolderOpen, FileCheck2, TriangleAlert } from "@lucide/svelte";
  import type { FileReport, CleanResult } from "./types";
  import { group, shockLine, sensitiveCount, BUCKET_LABELS, type Bucket } from "./classify";
  import { previewSrc, gpsText, openOnMap, extractThumbnail, revealFolder, openFile } from "./laver";

  let { report, result, busy }: { report: FileReport; result: CleanResult | null; busy: boolean } = $props();

  const grouped = $derived(group(report.tags));
  const shock = $derived(shockLine(report, grouped));
  const preview = $derived(previewSrc(report));
  const bucketOrder: Bucket[] = ["loc", "device", "dates", "software", "other"];
  const activeBuckets = $derived(bucketOrder.filter((b) => grouped.buckets[b].length > 0));

  const afterCount = $derived(result?.ok ? sensitiveCount(result.afterTags) : null);

  let expanded = $state(false);
  let copied = $state(false);
  let thumbSrc = $state<string | null>(null);
  let thumbLoading = $state(false);

  async function copyGps() {
    if (!report.gps) return;
    await navigator.clipboard.writeText(gpsText(report.gps));
    copied = true;
    setTimeout(() => (copied = false), 1400);
  }

  async function showThumbnail() {
    thumbLoading = true;
    thumbSrc = await extractThumbnail(report.path);
    thumbLoading = false;
  }

  const fmtBytes = (n: number) => (n < 1024 ? `${n} o` : `${(n / 1024).toFixed(1)} Ko`);
</script>

<article class="rounded-lg border border-edge bg-surface">
  <div class="flex gap-4 p-4">
    <!-- Aperçu -->
    <div class="grid size-16 shrink-0 place-items-center overflow-hidden rounded-md border border-edge bg-raised">
      {#if preview}
        <img src={preview} alt="" class="size-full object-cover" />
      {:else}
        <span class="text-[10px] font-medium uppercase tracking-wide text-dim">{report.fileType || "?"}</span>
      {/if}
    </div>

    <!-- Corps -->
    <div class="min-w-0 flex-1">
      <div class="flex items-baseline gap-2">
        <span class="truncate text-[14px] font-medium" title={report.fileName}>{report.fileName}</span>
        <span class="shrink-0 rounded bg-raised px-1.5 py-0.5 text-[10px] uppercase tracking-wide text-dim">
          {report.fileType || ext(report.path)}
        </span>
      </div>

      {#if report.error}
        <p class="mt-1 text-[13px] text-danger">{report.error}</p>
      {:else}
        <p class="mt-1 text-[13px] text-dim">{shock}</p>

        {#if activeBuckets.length > 0}
          <div class="mt-2.5 flex flex-wrap gap-1.5">
            {#each activeBuckets as b}
              <span
                class="rounded px-2 py-0.5 text-[11px] {b === 'loc'
                  ? 'bg-danger/12 text-danger'
                  : 'bg-raised text-text'}"
              >
                {BUCKET_LABELS[b]}
                <span class="text-dim">{grouped.buckets[b].length}</span>
              </span>
            {/each}
          </div>
        {/if}

        <!-- GPS : coordonnées en clair, jamais de carte intégrée -->
        {#if report.gps}
          <div class="mt-2.5 flex flex-wrap items-center gap-2 rounded-md bg-danger/8 px-2.5 py-1.5">
            <MapPin size={13} class="text-danger" />
            <span class="font-mono text-[12px]">{gpsText(report.gps)}</span>
            <button onclick={copyGps} class="flex items-center gap-1 text-[11px] text-dim hover:text-text">
              {#if copied}<Check size={12} class="text-accent" />Copié{:else}<Copy size={12} />Copier{/if}
            </button>
            <button onclick={() => report.gps && openOnMap(report.gps)} class="flex items-center gap-1 text-[11px] text-dim hover:text-text">
              <ExternalLink size={12} />Carte
            </button>
          </div>
        {/if}

        <!-- Vignette EXIF cachée -->
        {#if report.thumbnail}
          <div class="mt-2 flex items-center gap-2.5">
            {#if thumbSrc}
              <img src={thumbSrc} alt="vignette intégrée" class="h-12 rounded border border-edge" />
              <span class="text-[11px] leading-tight text-dim">Vignette intégrée — elle peut montrer<br />l'image d'avant recadrage.</span>
            {:else}
              <Image size={13} class="text-dim" />
              <span class="text-[11px] text-dim">Vignette intégrée cachée ({fmtBytes(report.thumbnail.bytes)})</span>
              <button onclick={showThumbnail} disabled={thumbLoading} class="text-[11px] text-accent hover:underline">
                {thumbLoading ? "…" : "Révéler"}
              </button>
            {/if}
          </div>
        {/if}

        {#if grouped.sensitiveCount > 0}
          <button onclick={() => (expanded = !expanded)} class="mt-2.5 flex items-center gap-1 text-[12px] text-dim hover:text-text">
            <ChevronDown size={13} class="transition-transform {expanded ? 'rotate-180' : ''}" />
            {expanded ? "Masquer" : `Voir les ${grouped.sensitiveCount} champs`}
          </button>
        {/if}

        {#if expanded}
          <div class="mt-2 space-y-2 border-t border-edge pt-2">
            {#each activeBuckets as b}
              <div>
                <div class="text-[10px] font-medium uppercase tracking-wide text-dim">{BUCKET_LABELS[b]}</div>
                <div class="mt-1 space-y-0.5">
                  {#each grouped.buckets[b] as tag}
                    <div class="flex gap-3 text-[12px]">
                      <span class="w-40 shrink-0 truncate text-dim" title="{tag.group}:{tag.name}">{tag.name}</span>
                      <span class="min-w-0 flex-1 truncate" title={tag.value}>{tag.value}</span>
                    </div>
                  {/each}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  </div>

  <!-- Bande de résultat après lavage -->
  {#if result}
    <div class="border-t border-edge px-4 py-2.5 text-[13px]">
      {#if result.ok}
        <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
          {#if afterCount === 0}
            <span class="flex items-center gap-1.5 text-accent"><FileCheck2 size={14} />Propre</span>
            <span class="text-dim">{grouped.sensitiveCount} → 0 champ sensible</span>
          {:else}
            <span class="flex items-center gap-1.5 text-danger"><TriangleAlert size={14} />{afterCount} champ(s) restant(s)</span>
            <span class="text-dim">{grouped.sensitiveCount} → {afterCount}</span>
          {/if}
          <span class="text-dim">·</span>
          <span class="truncate font-mono text-[12px] text-dim">{result.dstName}</span>
          <div class="grow"></div>
          <button onclick={() => result.dst && openFile(result.dst)} class="text-[12px] text-dim hover:text-text">Ouvrir</button>
          <button onclick={() => result.dst && revealFolder(result.dst)} class="flex items-center gap-1 text-[12px] text-dim hover:text-text">
            <FolderOpen size={12} />Dossier
          </button>
        </div>
      {:else}
        <span class="flex items-center gap-1.5 text-danger"><TriangleAlert size={14} />{result.error}</span>
      {/if}
    </div>
  {:else if busy}
    <div class="border-t border-edge px-4 py-2.5 text-[13px] text-dim">Lavage…</div>
  {/if}
</article>

<script lang="ts" module>
  function ext(path: string): string {
    const m = /\.([^.\\/]+)$/.exec(path);
    return m ? m[1].toUpperCase() : "?";
  }
</script>
