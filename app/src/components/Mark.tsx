/** The Zipkoo mark. `small` drops the zipper teeth for sizes under 32 px. */
export function Mark({ size = 24, small = false, inverse = false }: { size?: number; small?: boolean; inverse?: boolean }) {
  const bg = inverse ? "var(--ink)" : "var(--haldi)";
  const fg = inverse ? "var(--haldi)" : "var(--ink)";
  return (
    <svg width={size} height={size} viewBox="0 0 100 100" aria-hidden="true">
      <rect x="4" y="4" width="92" height="92" rx="26" fill={bg} />
      {small ? (
        <path d="M29 30H71L29 70H71" stroke={fg} strokeWidth="14" strokeLinecap="round" strokeLinejoin="round" fill="none" />
      ) : (
        <>
          <path d="M30 31H69M31 69H70" stroke={fg} strokeWidth="11" strokeLinecap="round" fill="none" />
          <line x1="67" y1="34" x2="33" y2="66" stroke={fg} strokeWidth="15" strokeDasharray="3.4 2.6" />
          <line x1="69" y1="32" x2="31" y2="68" stroke={bg} strokeWidth="1.8" />
          <circle cx="70" cy="31" r="8.5" fill={fg} />
          <circle cx="70" cy="31" r="3" fill={bg} />
        </>
      )}
    </svg>
  );
}
