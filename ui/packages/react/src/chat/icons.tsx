/** 会话组件用的线条图标（20×20 视框、描边随 currentColor），与原型 docs/ui/prototype 的 IC 表同形。 */

import type { ReactNode } from "react";

function Svg({ size = 18, width = 1.6, children }: { size?: number; width?: number; children: ReactNode }) {
  return (
    <svg
      viewBox="0 0 20 20"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={width}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

export const Icon = {
  plus: () => (
    <Svg>
      <path d="M10 4.5v11M4.5 10h11" />
    </Svg>
  ),
  sliders: () => (
    <Svg>
      <path d="M3.5 6.5h8M15 6.5h1.5M3.5 13.5h2M9 13.5h7.5" />
      <circle cx="13.2" cy="6.5" r="1.8" />
      <circle cx="7.2" cy="13.5" r="1.8" />
    </Svg>
  ),
  mic: () => (
    <Svg>
      <rect x="7.4" y="3" width="5.2" height="9" rx="2.6" />
      <path d="M4.8 9.8a5.2 5.2 0 0 0 10.4 0M10 15v2.4" />
    </Svg>
  ),
  up: () => (
    <Svg size={16} width={2}>
      <path d="M10 15.5v-11M5.5 9 10 4.5 14.5 9" />
    </Svg>
  ),
  stop: () => (
    <svg viewBox="0 0 20 20" width="12" height="12" aria-hidden="true">
      <rect x="4" y="4" width="12" height="12" rx="2.5" fill="currentColor" />
    </svg>
  ),
  down: () => (
    <Svg size={13} width={1.8}>
      <path d="m6 8 4 4 4-4" />
    </Svg>
  ),
  file: () => (
    <Svg size={16}>
      <path d="M6 2.8h5.4L15 6.4v10.8H6z" />
      <path d="M11.4 2.8v3.6H15" />
    </Svg>
  ),
  copy: () => (
    <Svg size={15}>
      <rect x="7" y="7" width="9.5" height="9.5" rx="2" />
      <path d="M13 7V5.2A1.7 1.7 0 0 0 11.3 3.5H5.2A1.7 1.7 0 0 0 3.5 5.2v6.1A1.7 1.7 0 0 0 5.2 13H7" />
    </Svg>
  ),
  check: () => (
    <Svg size={15} width={2}>
      <path d="m4.8 10.4 3.4 3.3L15.4 6.4" />
    </Svg>
  ),
  think: () => (
    <Svg size={15}>
      <path d="M10 3.4a4.1 4.1 0 0 0-4.1 4.1c0 1.6.8 2.7 1.6 3.5.5.5.9 1.2.9 2v1h3.2v-1c0-.8.4-1.5.9-2 .8-.8 1.6-1.9 1.6-3.5A4.1 4.1 0 0 0 10 3.4Z" />
      <path d="M8.4 16.6h3.2" />
    </Svg>
  ),
  book: () => (
    <Svg size={16}>
      <path d="M4 4.6A1.6 1.6 0 0 1 5.6 3H16v11.6H5.6A1.6 1.6 0 0 0 4 16.2z" />
      <path d="M4 16.2A1.6 1.6 0 0 0 5.6 17.8H16" />
    </Svg>
  ),
  fold: () => (
    <Svg size={16}>
      <path d="M6 8h8M6 12h8" />
      <path d="M3.5 4.5h13v11h-13z" />
    </Svg>
  ),
  shield: () => (
    <Svg size={15}>
      <path d="M10 3 4.5 5v4.6c0 3.4 2.3 6 5.5 7.4 3.2-1.4 5.5-4 5.5-7.4V5z" />
    </Svg>
  ),
  x: () => (
    <Svg size={16}>
      <path d="m6 6 8 8M14 6l-8 8" />
    </Svg>
  ),
  xs: () => (
    <Svg size={11} width={2}>
      <path d="m6.5 6.5 7 7M13.5 6.5l-7 7" />
    </Svg>
  ),
  image: () => (
    <Svg size={16}>
      <rect x="3" y="4" width="14" height="12" rx="2" />
      <circle cx="7.8" cy="8.4" r="1.4" />
      <path d="m4 15 4.4-4 3 2.6 2.4-2.2L17 14" />
    </Svg>
  ),
  at: () => (
    <Svg size={16}>
      <circle cx="10" cy="10" r="3" />
      <path d="M13 10v1.2a2.2 2.2 0 0 0 4.3 0V10a7.2 7.2 0 1 0-2.9 5.8" />
    </Svg>
  ),
  shot: () => (
    <Svg size={16}>
      <path d="M3 7V4.6A1.6 1.6 0 0 1 4.6 3H7M13 3h2.4A1.6 1.6 0 0 1 17 4.6V7M17 13v2.4a1.6 1.6 0 0 1-1.6 1.6H13M7 17H4.6A1.6 1.6 0 0 1 3 15.4V13" />
    </Svg>
  ),
  log: () => (
    <Svg size={16}>
      <path d="M5 3.5h10v13H5z" />
      <path d="M7.5 7h5M7.5 10h5M7.5 13h3" />
    </Svg>
  ),
  retry: () => (
    <Svg size={15}>
      <path d="M15.6 8.2A6 6 0 1 0 16 11" />
      <path d="M16.2 4.2v4h-4" />
    </Svg>
  ),
  globe: () => (
    <Svg size={16}>
      <circle cx="10" cy="10" r="7" />
      <path d="M3 10h14M10 3c2 2.2 2.8 4.6 2.8 7S12 14.8 10 17c-2-2.2-2.8-4.6-2.8-7S8 5.2 10 3Z" />
    </Svg>
  ),
  gauge: () => (
    <Svg size={16}>
      <path d="M3.5 13.5a6.5 6.5 0 1 1 13 0" />
      <path d="m10 13.5 3-4" />
    </Svg>
  ),
  pause: () => (
    <Svg size={13} width={2}>
      <path d="M7 5v10M13 5v10" />
    </Svg>
  ),
  play: () => (
    <svg viewBox="0 0 20 20" width="12" height="12" aria-hidden="true">
      <path d="M6 4.5v11l9-5.5z" fill="currentColor" />
    </svg>
  ),
  check16: () => (
    <Svg size={16} width={2}>
      <path d="m4.8 10.4 3.4 3.3L15.4 6.4" />
    </Svg>
  ),
  back: () => (
    <Svg size={15}>
      <path d="M15.5 10h-11M9 5.5 4.5 10 9 14.5" />
    </Svg>
  ),
};
