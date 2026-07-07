<script lang="ts">
  import { Droplets } from "@lucide/svelte";
</script>

<section class="flex h-full items-center justify-center p-10">
  <div
    class="w-full max-w-xl rounded-lg border border-dashed border-edge px-10 py-16 text-center"
  >
    <Droplets size={28} strokeWidth={1.5} class="mx-auto mb-4 text-dim" />
    <p class="text-[15px]">Dépose des photos ou des vidéos</p>
    <p class="mt-2 text-[13px] leading-relaxed text-dim">
      Inspection puis lavage des métadonnées, sur place.
      <br />
      Les originaux ne sont jamais modifiés.
    </p>
    <p class="mt-8 text-xs text-dim/60">Câblage en phase 2 — voir TODO.md</p>
  </div>
</section>
