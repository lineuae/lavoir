<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { Loader, FolderInput, Radio } from "@lucide/svelte";
  import type { Probe, Quality } from "$lib/types";
  import {
    probeUrl,
    startDownload,
    defaultDestination,
    pickDestination,
    getSettings,
    setSettings,
    QUALITIES,
    looksLikeUrl,
    formatDuration,
  } from "$lib/recuperer";
  import DownloadRow, { type Job } from "$lib/DownloadRow.svelte";

  let url = $state("");
  let probing = $state(false);
  let meta = $state<Probe | null>(null);
  let probeError = $state<string | null>(null);

  let quality = $state<Quality>("best");
  let wash = $state(true);
  let destination = $state("");
  let cookies = $state<string | null>(null);

  let jobs = $state<Job[]>([]);
  let inputEl: HTMLInputElement | undefined;

  const details = $derived.by(() => {
    if (!meta) return "";
    const parts: string[] = [];
    if (meta.source) parts.push(meta.source);
    const d = formatDuration(meta.durationSeconds);
    if (d) parts.push(d);
    if (meta.uploader) parts.push(meta.uploader);
    if (meta.maxHeight) parts.push(`jusqu'à ${meta.maxHeight}p`);
    return parts.join(" · ");
  });

  const canDownload = $derived(url.trim().length > 0 && !(meta?.isLive ?? false));
  const finishedCount = $derived(
    jobs.filter((j) => j.status === "done" || j.status === "error" || j.status === "cancelled").length,
  );

  async function runProbe() {
    const u = url.trim();
    if (!u || probing) return;
    probing = true;
    probeError = null;
    meta = null;
    try {
      meta = await probeUrl(u);
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

  function startJob() {
    const u = url.trim();
    if (!canDownload) return;

    const job: Job = {
      key: crypto.randomUUID(),
      id: null,
      label: meta?.title ?? u,
      source: meta?.source ?? null,
      audio: quality === "audio",
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

    startDownload({ url: u, quality, wash, destination, cookiesFromBrowser: cookies }, (ev) =>
      applyEvent(ref, ev),
    ).then(
      (id) => (ref.id = id),
      (err) => {
        ref.status = "error";
        ref.error = String(err);
      },
    );

    url = "";
    meta = null;
    probeError = null;
    inputEl?.focus();
  }

  function onEnter(e: KeyboardEvent) {
    if (e.key !== "Enter") return;
    e.preventDefault();
    if (meta) startJob();
    else runProbe();
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
  // Échap → efface le champ et la carte (annuler la saisie en cours).
  function onWindowKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "v" && document.activeElement !== inputEl) {
      e.preventDefault();
      pasteFromClipboard();
    } else if (e.key === "Escape" && (url || meta || probeError)) {
      e.preventDefault();
      url = "";
      meta = null;
      probeError = null;
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
      placeholder="Colle un lien puis Entrée…"
      class="w-full rounded-md border border-edge bg-surface px-4 py-3 text-[15px]
        placeholder:text-faint focus:border-accent focus:outline-none"
    />
    {#if probing}
      <Loader size={16} class="absolute right-3.5 top-1/2 -translate-y-1/2 animate-spin text-dim" />
    {/if}
  </div>

  {#if probeError}
    <p class="mt-3 text-[13px] text-danger">{probeError}</p>
  {:else if !meta}
    <p class="mt-3 text-[13px] leading-relaxed text-dim">
      Contenu public, ou accessible à ton propre compte. Le fichier arrive lavé, et l'app n'en
      garde aucune trace.
    </p>
  {/if}

  {#if meta}
    <div class="mt-3 rounded-md border border-edge bg-surface px-4 py-3">
      <p class="text-[13px] font-medium leading-snug">{meta.title}</p>
      {#if details}
        <p class="mt-1 text-[12px] text-dim">{details}</p>
      {/if}
      {#if meta.isLive}
        <p class="mt-2 flex items-center gap-1.5 text-[12px] text-danger">
          <Radio size={13} />Diffusion en direct — non prise en charge.
        </p>
      {/if}
    </div>
  {/if}

  <!-- Contrôles -->
  <div class="mt-5 flex flex-wrap items-center gap-x-4 gap-y-3">
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

    <label class="flex cursor-pointer items-center gap-2 text-[12px] text-dim">
      <input type="checkbox" bind:checked={wash} class="accent-accent" />
      Laver après téléchargement
    </label>

    <div class="grow"></div>

    <button
      onclick={startJob}
      disabled={!canDownload}
      class="rounded-md bg-accent px-3.5 py-1.5 text-[13px] font-medium text-bg transition-opacity
        hover:opacity-90 disabled:opacity-40"
    >
      {quality === "audio" ? "Récupérer l'audio" : "Récupérer"}
    </button>
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
