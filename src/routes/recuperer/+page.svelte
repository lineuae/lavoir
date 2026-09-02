<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { Loader, FolderInput, Radio, Image, Video, Check } from "@lucide/svelte";
  import type { ProbeResult, Probe, Listing, Entry, Quality, DownloadRequest } from "$lib/types";
  import {
    probeUrl,
    startDownload,
    defaultDestination,
    pickDestination,
    getSettings,
    setSettings,
    notifyDownloadDone,
    QUALITIES,
    looksLikeUrl,
    formatDuration,
  } from "$lib/recuperer";
  import DownloadRow, { type Job } from "$lib/DownloadRow.svelte";

  let url = $state("");
  let probing = $state(false);
  let result = $state<ProbeResult | null>(null);
  let probeError = $state<string | null>(null);

  let quality = $state<Quality>("best");
  let wash = $state(true);
  let destination = $state("");
  let cookies = $state<string | null>(null);

  // Sélection de la galerie (rangs `playlist_index` cochés).
  let selected = $state<number[]>([]);

  let jobs = $state<Job[]>([]);
  let inputEl: HTMLInputElement | undefined;

  const single = $derived<Probe | null>(result?.mode === "single" ? result : null);
  const listing = $derived<Listing | null>(result?.mode === "list" ? result : null);
  const isImage = $derived(single?.kind === "image");

  const details = $derived.by(() => {
    if (!single) return "";
    const parts: string[] = [];
    if (single.source) parts.push(single.source);
    if (single.kind === "image") parts.push("photo");
    const d = formatDuration(single.durationSeconds);
    if (d) parts.push(d);
    if (single.uploader) parts.push(single.uploader);
    if (single.kind !== "image" && single.maxHeight) parts.push(`jusqu'à ${single.maxHeight}p`);
    return parts.join(" · ");
  });

  const canDownloadSingle = $derived(single !== null && !single.isLive);
  const allSelected = $derived(
    listing !== null && listing.entries.length > 0 && selected.length === listing.entries.length,
  );
  const finishedCount = $derived(
    jobs.filter((j) => j.status === "done" || j.status === "error" || j.status === "cancelled").length,
  );

  async function runProbe() {
    const u = url.trim();
    if (!u || probing) return;
    probing = true;
    probeError = null;
    result = null;
    selected = [];
    try {
      const r = await probeUrl(u, cookies);
      // Rien à choisir : on n'impose pas une galerie vide.
      if (r.mode === "list" && r.entries.length === 0) {
        probeError = "Aucune story disponible ici pour le moment.";
        result = null;
      } else {
        result = r;
      }
    } catch (e) {
      probeError = String(e);
    } finally {
      probing = false;
    }
  }

  function applyEvent(job: Job, ev: import("$lib/types").DownloadEvent) {
    switch (ev.type) {
      case "queued":
        job.status = "queued";
        break;
      case "started":
        job.status = "downloading";
        break;
      case "progress":
        job.status = "downloading";
        job.downloaded = ev.downloaded;
        job.total = ev.total;
        job.speed = ev.speed;
        job.eta = ev.eta;
        break;
      case "postprocess":
        job.status = "postprocess";
        job.stage = ev.stage;
        break;
      case "washing":
        job.status = "washing";
        break;
      case "completed":
        job.status = "done";
        job.path = ev.path;
        job.name = ev.name;
        if (!document.hasFocus()) notifyDownloadDone(ev.name);
        break;
      case "failed":
        job.status = "error";
        job.error = ev.message;
        break;
      case "cancelled":
        job.status = "cancelled";
        break;
    }
  }

  function enqueue(req: DownloadRequest, label: string, source: string | null, audio: boolean) {
    const job: Job = {
      key: crypto.randomUUID(),
      id: null,
      label,
      source,
      audio,
      status: "queued",
      downloaded: null,
      total: null,
      speed: null,
      eta: null,
      stage: null,
      path: null,
      name: null,
      error: null,
    };
    jobs = [job, ...jobs];
    const ref = jobs[0];
    startDownload(req, (ev) => applyEvent(ref, ev)).then(
      (id) => (ref.id = id),
      (err) => {
        ref.status = "error";
        ref.error = String(err);
      },
    );
  }

  function base(): Omit<DownloadRequest, "url" | "source" | "kind" | "playlistItem"> {
    return { quality, wash, destination, cookiesFromBrowser: cookies };
  }

  function clearInput() {
    url = "";
    result = null;
    selected = [];
    probeError = null;
    inputEl?.focus();
  }

  function downloadSingle() {
    if (!single || !canDownloadSingle) return;
    const u = url.trim() || single.webpageUrl;
    enqueue(
      { ...base(), url: u, source: single.source, kind: single.kind, playlistItem: null },
      single.title,
      single.source,
      single.kind !== "image" && quality === "audio",
    );
    clearInput();
  }

  // Chaque story choisie devient un job qui vise l'item par son rang sur l'URL du
  // profil : yt-dlp ré-extrait un lien frais au moment du téléchargement, donc
  // l'ordre dans lequel on les lance n'a plus d'importance (fini les liens périmés).
  function downloadSelected() {
    if (!listing || selected.length === 0) return;
    const profileUrl = url.trim();
    const chosen = listing.entries
      .filter((e) => selected.includes(e.index))
      .sort((a, b) => a.index - b.index);
    for (const e of chosen) {
      enqueue(
        { ...base(), url: profileUrl, source: listing.source, kind: e.kind, playlistItem: e.index },
        e.title !== "Sans titre" ? e.title : `Story ${e.index}`,
        listing.source,
        e.kind !== "image" && quality === "audio",
      );
    }
    clearInput();
  }

  function toggle(index: number) {
    selected = selected.includes(index)
      ? selected.filter((i) => i !== index)
      : [...selected, index];
  }

  function toggleAll() {
    selected = allSelected ? [] : (listing?.entries.map((e) => e.index) ?? []);
  }

  function onEnter(e: KeyboardEvent) {
    if (e.key !== "Enter") return;
    e.preventDefault();
    if (single) downloadSingle();
    else if (!result) runProbe();
  }

  // Collage dans le champ : sonde automatiquement si ça ressemble à un lien.
  function onPaste() {
    queueMicrotask(() => {
      if (looksLikeUrl(url)) runProbe();
    });
  }

  async function pasteFromClipboard() {
    try {
      const text = await navigator.clipboard.readText();
      if (looksLikeUrl(text)) {
        url = text.trim();
        runProbe();
      }
    } catch {
      // presse-papiers indisponible : le collage natif dans le champ suffit.
    }
  }

  // Ctrl+V n'importe où dans la vue (champ non focalisé) → coller + sonder.
  // Échap → efface le champ et le résultat (annuler la saisie en cours).
  function onWindowKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "v" && document.activeElement !== inputEl) {
      e.preventDefault();
      pasteFromClipboard();
    } else if (e.key === "Escape" && (url || result || probeError)) {
      e.preventDefault();
      clearInput();
    }
  }

  async function changeDestination() {
    const picked = await pickDestination(destination);
    if (!picked) return;
    destination = picked;
    await setSettings({ ...(await getSettings()), destination: picked });
  }

  async function resetDestination() {
    destination = await defaultDestination();
    await setSettings({ ...(await getSettings()), destination: null });
  }

  function clearFinished() {
    jobs = jobs.filter(
      (j) => j.status !== "done" && j.status !== "error" && j.status !== "cancelled",
    );
  }

  function entryLine(e: Entry): string {
    const parts: string[] = [e.kind === "image" ? "Photo" : "Vidéo"];
    const d = formatDuration(e.durationSeconds);
    if (e.kind !== "image" && d) parts.push(d);
    if (e.title !== "Sans titre") parts.push(e.title);
    return parts.join(" · ");
  }

  onMount(async () => {
    const s = await getSettings();
    cookies = s.cookiesFromBrowser;
    destination = s.destination ?? (await defaultDestination());
    inputEl?.focus();
    window.addEventListener("keydown", onWindowKey);
  });
  onDestroy(() => window.removeEventListener("keydown", onWindowKey));
</script>

<section class="mx-auto max-w-2xl px-8 py-10">
  <div class="relative">
    <input
      bind:this={inputEl}
      bind:value={url}
      onpaste={onPaste}
      onkeydown={onEnter}
      type="url"
      placeholder="Colle un lien, ou un profil pour voir ses stories…"
      class="w-full rounded-md border border-edge bg-surface px-4 py-3 text-[15px]
        placeholder:text-faint focus:border-accent focus:outline-none"
    />
    {#if probing}
      <Loader size={16} class="absolute right-3.5 top-1/2 -translate-y-1/2 animate-spin text-dim" />
    {/if}
  </div>

  {#if probeError}
    <p class="mt-3 text-[13px] text-danger">{probeError}</p>
  {:else if !result}
    <p class="mt-3 text-[13px] leading-relaxed text-dim">
      Un lien précis, ou le profil de quelqu'un pour choisir parmi ses stories. Le fichier arrive
      lavé, et l'app n'en garde aucune trace.
    </p>
  {/if}

  {#if single}
    <div class="mt-3 rounded-md border border-edge bg-surface px-4 py-3">
      <p class="text-[13px] font-medium leading-snug">{single.title}</p>
      {#if details}
        <p class="mt-1 text-[12px] text-dim">{details}</p>
      {/if}
      {#if single.isLive}
        <p class="mt-2 flex items-center gap-1.5 text-[12px] text-danger">
          <Radio size={13} />Diffusion en direct — non prise en charge.
        </p>
      {/if}
    </div>
  {/if}

  {#if listing}
    <div class="mt-3">
      <div class="mb-2 flex items-baseline gap-3">
        <p class="text-[12px] text-dim">
          {listing.entries.length} {listing.entries.length > 1 ? "stories" : "story"}
          {#if listing.title}· {listing.title}{/if}
        </p>
        <div class="grow"></div>
        <button onclick={toggleAll} class="text-[12px] text-dim hover:text-text">
          {allSelected ? "Tout décocher" : "Tout sélectionner"}
        </button>
      </div>
      <div class="max-h-80 space-y-1 overflow-y-auto pr-1">
        {#each listing.entries as e (e.index)}
          {@const on = selected.includes(e.index)}
          <button
            onclick={() => toggle(e.index)}
            class="flex w-full items-center gap-3 rounded-md border px-3 py-2 text-left transition-colors
              {on ? 'border-accent/60 bg-raised' : 'border-edge hover:bg-surface'}"
          >
            <span
              class="flex size-4 shrink-0 items-center justify-center rounded border
                {on ? 'border-accent bg-accent text-bg' : 'border-edge'}"
            >
              {#if on}<Check size={12} strokeWidth={3} />{/if}
            </span>
            {#if e.kind === "image"}
              <Image size={14} class="shrink-0 text-faint" />
            {:else}
              <Video size={14} class="shrink-0 text-faint" />
            {/if}
            <span class="min-w-0 truncate text-[13px]">{entryLine(e)}</span>
          </button>
        {/each}
      </div>
    </div>
  {/if}

  <!-- Contrôles -->
  <div class="mt-5 flex flex-wrap items-center gap-x-4 gap-y-3">
    {#if !isImage}
      <div class="flex rounded-md border border-edge p-0.5">
        {#each QUALITIES as q}
          <button
            onclick={() => (quality = q.id)}
            class="rounded px-2.5 py-1 text-[12px] transition-colors
              {quality === q.id ? 'bg-raised text-text' : 'text-dim hover:text-text'}"
          >
            {q.label}
          </button>
        {/each}
      </div>
    {/if}

    <label class="flex cursor-pointer items-center gap-2 text-[12px] text-dim">
      <input type="checkbox" bind:checked={wash} class="accent-accent" />
      Laver après téléchargement
    </label>

    <div class="grow"></div>

    {#if listing}
      <button
        onclick={downloadSelected}
        disabled={selected.length === 0}
        class="rounded-md bg-accent px-3.5 py-1.5 text-[13px] font-medium text-bg transition-opacity
          hover:opacity-90 disabled:opacity-40"
      >
        {selected.length > 0 ? `Récupérer la sélection (${selected.length})` : "Choisis des stories"}
      </button>
    {:else if single}
      <button
        onclick={downloadSingle}
        disabled={!canDownloadSingle}
        class="rounded-md bg-accent px-3.5 py-1.5 text-[13px] font-medium text-bg transition-opacity
          hover:opacity-90 disabled:opacity-40"
      >
        {isImage ? "Récupérer la photo" : quality === "audio" ? "Récupérer l'audio" : "Récupérer"}
      </button>
    {:else}
      <button
        onclick={runProbe}
        disabled={url.trim().length === 0 || probing}
        class="rounded-md bg-accent px-3.5 py-1.5 text-[13px] font-medium text-bg transition-opacity
          hover:opacity-90 disabled:opacity-40"
      >
        Analyser le lien
      </button>
    {/if}
  </div>

  <!-- Destination -->
  <div class="mt-3 flex items-center gap-2 text-[12px] text-dim">
    <FolderInput size={13} class="shrink-0" />
    <span class="shrink-0">Vers</span>
    <span class="min-w-0 truncate font-mono text-text/70" title={destination}>{destination}</span>
    <button onclick={changeDestination} class="shrink-0 hover:text-text">Modifier</button>
    <button onclick={resetDestination} class="shrink-0 hover:text-text">Défaut</button>
  </div>

  {#if jobs.length > 0}
    <div class="mt-8">
      <div class="mb-2 flex items-center gap-3">
        <h2 class="text-[12px] font-medium uppercase tracking-wide text-dim">Téléchargements</h2>
        <div class="grow"></div>
        {#if finishedCount > 0}
          <button onclick={clearFinished} class="text-[12px] text-dim hover:text-text">
            Effacer terminés
          </button>
        {/if}
      </div>
      <div class="space-y-2">
        {#each jobs as job (job.key)}
          <DownloadRow {job} />
        {/each}
      </div>
    </div>
  {/if}
</section>
