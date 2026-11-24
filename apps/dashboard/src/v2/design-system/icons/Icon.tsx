import type { CSSProperties } from "react";

const paths = {
  files: "M8 3h9l4 4v13H8z M17 3v5h4 M4 7v14h12",
  search: "M20 20l-5-5 M17 10a7 7 0 1 1-14 0 7 7 0 0 1 14 0",
  git: "M6 5v13a3 3 0 0 0 3 3 M6 8c0 6 12 0 12 6v3 M3 3h6v5H3z M15 17h6v5h-6z",
  agents:
    "M8 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8 M2 21v-3c0-5 12-5 12 0v3 M17 4a4 4 0 0 1 0 8 M18 16c3 0 4 2 4 5",
  memory: "M4 3h13a3 3 0 0 1 3 3v15H6a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3 M7 7h9 M7 11h7 M4 17h16",
  settings: "M4 6h16 M4 12h16 M4 18h16 M8 3v6 M16 9v6 M10 15v6",
  extensions: "M3 4h7v6h5V4h6v7h-6v5h6v5h-8v-5H8v5H3v-8h5V8H3z",
  chevron: "M9 5l7 7-7 7",
  down: "M5 9l7 7 7-7",
  close: "M6 6l12 12 M18 6L6 18",
  clear: "M16 3l6 6-10 10H6l-4-4z M9 8l8 8 M11 19h11",
  plus: "M12 5v14 M5 12h14",
  arrow: "M12 19V5 M5 12l7-7 7 7",
  check: "M5 12l4 4L19 6",
  warning: "M12 3L2 21h20z M12 9v5 M12 17v1",
  error: "M8 3h8l5 5v8l-5 5H8l-5-5V8z M12 7v6 M12 16v1",
  info: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18 M12 11v6 M12 7v1",
  clock: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18 M12 7v5l4 2",
  play: "M8 4l12 8-12 8z",
  pause: "M7 5v14 M17 5v14",
  stop: "M5 5h14v14H5z",
  step: "M5 5l10 7L5 19z M19 5v14",
  terminal: "M4 5l7 7-7 7 M12 19h8",
  code: "M8 6l-6 6 6 6 M16 6l6 6-6 6 M14 3l-4 18",
  file: "M5 3h10l4 4v14H5z M14 3v5h5 M8 12h8 M8 16h6",
  folder: "M3 5h7l2 3h9v13H3z",
  bell: "M5 17V9a7 7 0 0 1 14 0v8l2 2H3z M9 22h6",
  moon: "M19 15A9 9 0 0 1 9 3a9 9 0 1 0 12 12z",
  sun: "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8 M12 1v3 M12 20v3 M1 12h3 M20 12h3 M4 4l2 2 M18 18l2 2 M4 20l2-2 M18 6l2-2",
  layout: "M3 4h18v16H3z M8 4v16 M8 15h13 M17 4v11",
  split: "M3 4h18v16H3z M12 4v16",
  collapse: "M3 4h18v16H3z M8 4v16 M16 9l-3 3 3 3",
  more: "M5 11v2 M12 11v2 M19 11v2",
  checkpoint: "M12 3l8 5v8l-8 5-8-5V8z M8 12l3 3 5-6",
  history: "M3 11a9 9 0 1 1 3 8 M3 3v8h8 M12 7v6l4 2",
  refresh: "M20 7a8 8 0 0 0-14-2L3 8 M3 3v5h5 M4 17a8 8 0 0 0 14 2l3-3 M21 21v-5h-5",
  link: "M9 15l6-6 M7 17l-2 2a4 4 0 0 1-5-5l5-5a4 4 0 0 1 5 0 M17 7l2-2a4 4 0 0 1 5 5l-5 5a4 4 0 0 1-5 0",
} as const;

export type IconName = keyof typeof paths;

export function Icon({
  name,
  size = 16,
  style,
}: {
  name: IconName;
  size?: number;
  style?: CSSProperties;
}) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      style={style}
    >
      <path d={paths[name]} />
    </svg>
  );
}
