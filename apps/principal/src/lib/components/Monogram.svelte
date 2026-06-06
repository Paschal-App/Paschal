<script lang="ts">
  // The Paschal mark: a candle flame held within the seal ring. Candle Gold
  // flame, Burgundy ember at its heart, Ink wick and candle ledge. On dark
  // surfaces (inverse) the ring/wick/ember invert to Parchment; the flame
  // stays gold so it still reads as a flame.
  type Props = { size?: number; inverse?: boolean };
  let { size = 32, inverse = false }: Props = $props();

  const ink = $derived(inverse ? '#F4ECD8' : '#0E1B33');
  const flame = '#C8861E';
  const ember = $derived(inverse ? '#F4ECD8' : '#6B1A1A');
</script>

<svg
  xmlns="http://www.w3.org/2000/svg"
  viewBox="0 0 200 200"
  width={size}
  height={size}
  role="img"
  aria-label="Paschal"
>
  <title>Paschal</title>
  <circle cx="100" cy="100" r="92" fill="none" stroke={ink} stroke-width="1.25" />
  <circle cx="100" cy="100" r="86" fill="none" stroke={ink} stroke-width="0.4" opacity="0.55" />
  <!-- Flame body -->
  <path
    d="M 100 44 C 118 74 130 92 130 112 A 30 30 0 1 1 70 112 C 70 90 84 72 100 44 Z"
    fill={flame}
  />
  <!-- Inner ember -->
  <path
    d="M 100 96 C 109 110 113 120 106 130 A 12 12 0 1 1 94 130 C 90 122 92 110 100 96 Z"
    fill={ember}
  />
  <!-- Wick + candle ledge -->
  <rect x="97" y="140" width="6" height="18" fill={ink} />
  <rect x="82" y="158" width="36" height="4" fill={ink} />
</svg>
