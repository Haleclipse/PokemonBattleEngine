/** Accessible type chips. Text is always present; color is an extra cue, not the only one. */
export const TYPE_COLOR: Record<string, string> = {
  Normal: "#8d8d78",
  Fire: "#e25822",
  Water: "#3d7ec9",
  Electric: "#c9a227",
  Grass: "#3e9a4a",
  Ice: "#4aa8b5",
  Fighting: "#b44444",
  Poison: "#9a4eab",
  Ground: "#b5924a",
  Flying: "#7484c4",
  Psychic: "#e05b84",
  Bug: "#7aa342",
  Rock: "#a08b55",
  Ghost: "#6a548f",
  Dragon: "#4f63c4",
  Dark: "#5c5668",
  Steel: "#7d93a8",
  Fairy: "#d47aa6",
  Stellar: "#d7c56a",
}

export function typeStyle(english: string): { background: string; color: string } {
  const background = TYPE_COLOR[english] ?? "#6e6e6e"
  const color = english === "Electric" || english === "Ice" || english === "Steel" || english === "Stellar" ? "#1a1a1a" : "#f7f7f7"
  return { background, color }
}

export function hpBand(percent: number): "high" | "mid" | "low" {
  if (percent > 50) return "high"
  if (percent > 20) return "mid"
  return "low"
}
