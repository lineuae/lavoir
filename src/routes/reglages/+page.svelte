<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getVersion } from "@tauri-apps/api/app";
  import { RefreshCw } from "@lucide/svelte";
  import type { Settings, UpdateResult } from "$lib/types";
  import { getSettings, setSettings, updateYtdlp, launchUpdateStatus } from "$lib/recuperer";

  type ToolStatus = { name: string; version: string | null; error: string | null };

  let appVersion = $state("");
  let tools = $state<ToolStatus[] | null>(null);
  let settings = $state<Settings | null>(null);

  let updating = $state(false);
  let updateMsg = $state<string | null>(null);
  let launchStatus = $state<UpdateResult | null>(null);

  const browsers = [
    { value: null, label: "Aucun" },
    { value: "firefox", label: "Firefox" },
    { value: "chrome", label: "Chrome" },
    { value: "edge", label: "Edge" },
  ];

  async function save(patch: Partial<Settings>) {
    if (!settings) return;
    settings = { ...settings, ...patch };
    await setSettings(settings);
  }

  async function runUpdate() {
    updating = true;
    updateMsg = null;
    try {
      const result = await updateYtdlp();
      updateMsg = result.message;
      tools = await invoke<ToolStatus[]>("sidecar_versions");
    } finally {
      updating = false;
    }
  }

  onMount(async () => {
    appVersion = await getVersion();
    tools = await invoke<ToolStatus[]>("sidecar_versions");
    settings = await getSettings();
    launchStatus = await launchUpdateStatus();
  });
</script>

<section class="mx-auto max-w-2xl px-10 py-12">
  <h1 class="text-[15px] font-semibold">Réglages</h1>
  <p class="mt-1 text-[13px] text-dim">lavoir {appVersion}</p>

  <!-- Moteurs -->
  <h2 class="mt-10 text-[13px] font-medium text-dim">Moteurs embarqués</h2>
  <div class="mt-3 divide-y divide-edge rounded-md border border-edge bg-surface">
    {#if tools === null}
      <p class="px-4 py-3 text-[13px] text-dim">Vérification…</p>
    {:else}
      {#each tools as tool}
        <div class="flex items-center justify-between px-4 py-3">
          <span class="text-[13px]">{tool.name}</span>
          <div class="flex items-center gap-3">
            {#if tool.version}
              <span class="font-mono text-[12px] text-accent">{tool.version}</span>
            {:else}
              <span class="max-w-[50%] text-right text-[12px] text-danger">{tool.error}</span>
            {/if}
            {#if tool.name === "yt-dlp"}
              <button
                onclick={runUpdate}
                disabled={updating}
                class="flex items-center gap-1.5 rounded border border-edge px-2 py-1 text-[12px]
                  text-dim transition-colors hover:text-text disabled:opacity-40"
              >
                <RefreshCw size={12} class={updating ? "animate-spin" : ""} />
                {updating ? "Mise à jour…" : "Mettre à jour"}
              </button>
            {/if}
          </div>
        </div>
      {/each}
    {/if}
  </div>
  {#if updateMsg}
    <p class="mt-2 text-[12px] text-dim">{updateMsg}</p>
  {:else if launchStatus}
    <p class="mt-2 text-[12px] text-dim">Au démarrage : {launchStatus.message}</p>
  {:else}
    <p class="mt-2 text-[12px] leading-relaxed text-faint">
      Embarqués avec l'app, aucune installation requise. yt-dlp vieillit vite — mets-le à jour si un
      site cesse de fonctionner.
    </p>
  {/if}

  <!-- Téléchargement -->
  {#if settings}
    <h2 class="mt-10 text-[13px] font-medium text-dim">Téléchargement</h2>
    <div class="mt-3 space-y-4 rounded-md border border-edge bg-surface px-4 py-4">
      <div>
        <div class="flex items-center justify-between gap-4">
          <label for="cookies" class="text-[13px]">Utiliser ma session navigateur</label>
          <select
            id="cookies"
            value={settings.cookiesFromBrowser}
            onchange={(e) => save({ cookiesFromBrowser: e.currentTarget.value || null })}
            class="rounded border border-edge bg-raised px-2 py-1 text-[12px] focus:border-accent focus:outline-none"
          >
            {#each browsers as b}
              <option value={b.value}>{b.label}</option>
            {/each}
          </select>
        </div>
        <p class="mt-1.5 text-[11px] leading-relaxed text-faint">
          Pour les contenus visibles seulement une fois connecté à ton propre compte. Sous Windows,
          Chrome et Edge chiffrent leurs cookies (app-bound) : Firefox est le choix fiable.
        </p>
      </div>

      <label class="flex cursor-pointer items-center justify-between gap-4">
        <span class="text-[13px]">Vérifier les mises à jour de yt-dlp au démarrage</span>
        <input
          type="checkbox"
          checked={settings.checkYtdlpOnLaunch}
          onchange={(e) => save({ checkYtdlpOnLaunch: e.currentTarget.checked })}
          class="accent-accent"
        />
      </label>
    </div>
  {/if}
</section>
