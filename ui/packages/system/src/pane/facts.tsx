/** Facts in the right column: a label and its value, the empty ones left out. */

export function Facts({ rows }: { rows: [string, string][] }) {
  return (
    <dl className="facts">
      {rows
        .filter(([, value]) => value)
        .map(([label, value]) => (
          <div key={label} className="fact">
            <dt>{label}</dt>
            <dd title={value}>{value}</dd>
          </div>
        ))}
    </dl>
  );
}
