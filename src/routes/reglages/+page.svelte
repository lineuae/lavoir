<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getVersion } from "@tauri-apps/api/app";

  type ToolStatus = {
    name: string;
    version: string | null;
    error: string | null;
  };

  let appVersion = $state("");
  let tools = $state<ToolStatus[] | null>(null);

  onMount(async () => {
    appVersion = await getVersion();
    tools = await invoke<ToolStatus[]>("sidecar_versions");
  });
</script>

<section class="mx-auto max-w-2xl px-10 py-14">
  <h1 class="text-[15px] font-semibold">Réglages</h1>
  <p class="mt-1 text-[13px] text-dim">lavoir {appVersion}</p>

  <h2 class="mt-10 text-[13px] font-medium text-dim">Moteurs embarqués</h2>
  <div class="mt-3 divide-y divide-edge rounded-md border border-edge bg-surface">
    {#if tools === null}
      <p class="px-4 py-3 text-[13px] text-dim">Vérification…</p>
    {:else}
      {#each tools as tool}
        <div class="flex items-baseline justify-between px-4 py-3">
          <span class="text-[13px]">{tool.name}</span>
          {#if tool.version}
            <span class="font-mono text-[12px] text-accent">{tool.version}</span>
          {:else}
            <span class="max-w-[60%] text-right text-[12px] text-danger">{tool.error}</span>
          {/if}
        </div>
      {/each}
    {/if}
  </div>
  <p class="mt-3 text-xs leading-relaxed text-dim/60">
    Embarqués avec l'app, aucune installation requise. Mise à jour de yt-dlp : phase 4.
  </p>
</section>
