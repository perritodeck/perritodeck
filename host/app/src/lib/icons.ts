// Built-in key icons (Tabler Icons, MIT), drawn as SVG and turned into PNGs for the host.

import { ICONS } from "./icons.generated";

export const ICON_NAMES = Object.keys(ICONS);

/** The icon as an SVG string in `color`. */
export function iconSvg(name: string, color: string, size = 24): string {
  const nodes = ICONS[name] ?? [];
  const body = nodes
    .map(([tag, attrs]) => `<${tag} ${Object.entries(attrs).map(([k, v]) => `${k}="${v}"`).join(" ")}/>`)
    .join("");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="${color}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${body}</svg>`;
}

/** Draw the icon into a PNG (base64, no data: prefix) for the host to keep. */
export async function iconPngBase64(name: string, color: string, size = 192): Promise<string> {
  const img = new Image();
  img.src = "data:image/svg+xml;charset=utf-8," + encodeURIComponent(iconSvg(name, color, size));
  await img.decode();
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = size;
  canvas.getContext("2d")!.drawImage(img, 0, 0, size, size);
  return canvas.toDataURL("image/png").split(",")[1];
}
