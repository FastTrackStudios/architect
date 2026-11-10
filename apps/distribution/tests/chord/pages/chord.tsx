import { h, useEmbed } from "architect";

export default function Chord() {
  const embed = useEmbed();
  return h("strong", { class: "chord" }, `chord ${embed?.source?.trim() ?? ""}`);
}
