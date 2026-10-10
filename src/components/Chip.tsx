type Props = { kind?: "tier" | "pace" | "warn" | "muted"; children: string };

export default function Chip({ kind = "tier", children }: Props) {
  return <span className={`chip ${kind === "tier" ? "" : `chip--${kind}`} num`}>{children}</span>;
}
