<template>
  <svg
    :class="className"
    viewBox="0 0 256 256"
    fill="none"
    xmlns="http://www.w3.org/2000/svg"
    aria-hidden="true"
  >
    <rect x="8" y="8" width="240" height="240" rx="54" fill="url(#pony-bg)" />
    <rect x="8.75" y="8.75" width="238.5" height="238.5" rx="53.25" stroke="rgba(255,255,255,0.28)" stroke-width="1.5" />
    <path d="M116 60L134 28L149 68L116 60Z" fill="#4E3D30" />
    <path d="M98 82L116 54L129 92L98 82Z" fill="#4E3D30" opacity="0.9" />
    <ellipse cx="121" cy="104" rx="45" ry="42" fill="#4E3D30" />
    <rect x="145" y="100" width="47" height="37" rx="18.5" fill="#4E3D30" />
    <rect x="94" y="136" width="53" height="77" rx="24" fill="#4E3D30" />
    <path d="M91 137L144 109L165 131L115 197L91 137Z" fill="url(#pony-cut)" />
    <path d="M130 213L146 190L141 213H130Z" fill="url(#pony-cut)" />
    <circle cx="169" cy="109" r="4.25" fill="#F8F4EE" />
    <path
      d="M191 63C174.539 48.9536 152.938 41 130.457 41C112.935 41 95.9496 45.8354 81 54.9735"
      stroke="#E7BB82"
      stroke-width="11"
      stroke-linecap="round"
      opacity="0.34"
    />
    <path
      d="M191 63C174.539 48.9536 152.938 41 130.457 41C112.935 41 95.9496 45.8354 81 54.9735"
      stroke="#C97F3E"
      stroke-width="7"
      stroke-linecap="round"
    />
    <rect x="175" y="53" width="24" height="12" rx="6" fill="#C97F3E" />
    <defs>
      <linearGradient id="pony-bg" x1="128" y1="8" x2="128" y2="248" gradientUnits="userSpaceOnUse">
        <stop stop-color="#F8F3EA" />
        <stop offset="1" stop-color="#ECE4D6" />
      </linearGradient>
      <linearGradient id="pony-cut" x1="128" y1="109" x2="128" y2="213" gradientUnits="userSpaceOnUse">
        <stop stop-color="#F8F3EA" />
        <stop offset="1" stop-color="#ECE4D6" />
      </linearGradient>
    </defs>
  </svg>
</template>

<script setup lang="ts">
withDefaults(
  defineProps<{
    className?: string;
  }>(),
  {
    className: "h-6 w-6"
  }
);
</script>
