import { fillHeight, fillY, levelFor } from "../lib/vessel";
import { vesselAriaLabel, vesselValue } from "../lib/usageText";

type Props = {
  name: string;
  // Window length shown under the name ("5 hours"); omit for per-model vessels.
  hint?: string;
  // null = no data yet: a dashed empty vessel, never 0%.
  percent: number | null;
  caption: string | null;
  resetsIn?: string;
};

// One limit as a glass vessel that fills up (viewBox 0 0 40 70).
export default function Vessel({ name, hint, percent, caption, resetsIn }: Props) {
  const level = levelFor(percent);
  return (
    <div className={`vessel vessel--${level}`}>
      <svg
        className="vessel__svg"
        viewBox="0 0 40 70"
        role="img"
        aria-label={vesselAriaLabel(name, percent, resetsIn)}
      >
        <rect className="vessel__glass" x="2" y="2" width="36" height="66" rx="8" />
        {percent !== null && percent > 0 && (
          <rect
            className="vessel__liquid"
            x="5"
            y={fillY(percent)}
            width="30"
            height={fillHeight(percent)}
            rx="5"
          />
        )}
      </svg>
      <span className="vessel__value num">{vesselValue(percent)}</span>
      <span className="vessel__name">{name}</span>
      {hint && <span className="vessel__hint">{hint}</span>}
      {caption && <span className="vessel__hint">{caption}</span>}
    </div>
  );
}
