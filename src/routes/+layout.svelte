<script lang="ts">
  import "../app.css";
  import { page } from "$app/state";
  import { Droplets, ArrowDownToLine, Settings2 } from "@lucide/svelte";

  let { children } = $props();

  const tabs = [
    { href: "/", label: "Laver", icon: Droplets },
    { href: "/recuperer", label: "Récupérer", icon: ArrowDownToLine },
  ];

  const isActive = (href: string) => page.url.pathname === href;
</script>

<div class="flex h-screen flex-col">
  <header class="flex h-[52px] shrink-0 items-center gap-7 border-b border-edge px-5">
    <span class="text-[15px] font-semibold tracking-tight">lavoir</span>

    <nav class="flex h-full items-center gap-1">
      {#each tabs as tab}
        <a
          href={tab.href}
          class="flex h-full items-center gap-2 border-b-2 px-3 text-[13px] transition-colors
            {isActive(tab.href)
            ? 'border-accent text-text'
            : 'border-transparent text-dim hover:text-text'}"
        >
          <tab.icon size={15} strokeWidth={1.8} />
          {tab.label}
        </a>
      {/each}
    </nav>

    <div class="grow"></div>

    <a
      href="/reglages"
      title="Réglages"
      class="rounded-md p-2 transition-colors {isActive('/reglages')
        ? 'text-text'
        : 'text-dim hover:text-text'}"
    >
      <Settings2 size={16} strokeWidth={1.8} />
    </a>
  </header>

  <main class="min-h-0 flex-1 overflow-y-auto">
    {@render children()}
  </main>
</div>
