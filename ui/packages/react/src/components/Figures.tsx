/**
 * 两种画数字的小件，给汇总页用：一张点表头就能排序的表，一排按天的竖条。不带图表库，颜色只用令牌。
 */

import { useState, type ReactNode } from "react";

/** 表的一列：`value` 是排序用的值，`render` 是画出来的样子（缺省照 `value`）。 */
export interface FigureColumn<R> {
  id: string;
  label: string;
  value: (row: R) => number | string;
  render?: (row: R) => ReactNode;
  /** 悬停提示。 */
  title?: (row: R) => string | undefined;
  /** 数字列：靠右，先按从大到小排。 */
  numeric?: boolean;
}

export interface FigureTableProps<R> {
  rows: readonly R[];
  columns: readonly FigureColumn<R>[];
  rowKey: (row: R) => string;
  /** 一开始按哪一列、哪个方向排；缺省按第一个数字列从大到小。 */
  sort?: { column: string; descending: boolean };
  testId?: string;
  /** 一行的 `data-key`。 */
  dataKey?: (row: R) => string;
}

/** 点表头按那一列排，再点一次反过来；`aria-sort` 标着当前的列。 */
export function FigureTable<R>({ rows, columns, rowKey, sort, testId, dataKey }: FigureTableProps<R>) {
  const first = columns.find((c) => c.numeric) ?? columns[0];
  const [by, setBy] = useState(sort ?? { column: first?.id ?? "", descending: Boolean(first?.numeric) });
  const column = columns.find((c) => c.id === by.column) ?? first;
  const sorted = column ? [...rows].sort((a, b) => compare(column.value(a), column.value(b)) * (by.descending ? -1 : 1)) : [...rows];
  const pick = (c: FigureColumn<R>) => setBy((now) => (now.column === c.id ? { column: c.id, descending: !now.descending } : { column: c.id, descending: Boolean(c.numeric) }));
  return (
    <table className="hx-figures" data-testid={testId}>
      <thead>
        <tr>
          {columns.map((c) => (
            <th
              key={c.id}
              className={c.numeric ? "num" : undefined}
              aria-sort={column?.id === c.id ? (by.descending ? "descending" : "ascending") : undefined}
              data-column={c.id}
            >
              <button type="button" onClick={() => pick(c)}>
                {c.label}
                {column?.id === c.id ? <span aria-hidden>{by.descending ? " ↓" : " ↑"}</span> : null}
              </button>
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {sorted.map((row) => (
          <tr key={rowKey(row)} data-key={dataKey?.(row)}>
            {columns.map((c) => (
              <td key={c.id} className={c.numeric ? "num" : undefined} title={c.title?.(row)} data-column={c.id}>
                {c.render ? c.render(row) : String(c.value(row))}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function compare(a: number | string, b: number | string): number {
  if (typeof a === "number" && typeof b === "number") return a - b;
  return String(a).localeCompare(String(b));
}

/** 一根竖条：`label` 写在轴下（只写头、中、尾三处），`title` 是悬停提示。 */
export interface Bar {
  key: string;
  label: string;
  value: number;
  title: string;
}

/** 一排竖条，高度按最大的那根折算；值为 0 的只留一条底线。 */
export function Bars({ bars, label, testId }: { bars: readonly Bar[]; label: string; testId?: string }) {
  const max = Math.max(0, ...bars.map((b) => b.value));
  const shown = new Set([0, Math.floor((bars.length - 1) / 2), bars.length - 1]);
  return (
    <figure className="hx-bars" data-testid={testId} aria-label={label}>
      <div className="plot" role="list">
        {bars.map((b) => (
          <div key={b.key} className="col" role="listitem" title={b.title} aria-label={b.title} data-key={b.key} data-value={b.value}>
            <i style={{ height: max > 0 && b.value > 0 ? `${Math.max(2, (b.value / max) * 100)}%` : undefined }} className={b.value > 0 ? undefined : "zero"} />
          </div>
        ))}
      </div>
      <div className="axis" aria-hidden>
        {bars.map((b, i) => (
          <span key={b.key}>{shown.has(i) ? b.label : ""}</span>
        ))}
      </div>
    </figure>
  );
}
