/** 按行的最小差异（LCS）：工具行里编辑类的 diff、应用里新旧对照都用。文本小（几十到几百行），O(n·m) 足够。 */

export interface DiffLine {
  op: "same" | "add" | "del";
  text: string;
}

export function lineDiff(oldText: string, newText: string): DiffLine[] {
  const a = oldText.split("\n");
  const b = newText.split("\n");
  const n = a.length;
  const m = b.length;
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i]![j] = a[i] === b[j] ? lcs[i + 1]![j + 1]! + 1 : Math.max(lcs[i + 1]![j]!, lcs[i]![j + 1]!);
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ op: "same", text: a[i]! });
      i++;
      j++;
    } else if (lcs[i + 1]![j]! >= lcs[i]![j + 1]!) {
      out.push({ op: "del", text: a[i]! });
      i++;
    } else {
      out.push({ op: "add", text: b[j]! });
      j++;
    }
  }
  while (i < n) out.push({ op: "del", text: a[i++]! });
  while (j < m) out.push({ op: "add", text: b[j++]! });
  return out;
}

/** 只留改动附近的几行（其余折成"…"） */
export function hunks(lines: DiffLine[], context = 2): (DiffLine | { op: "gap"; text: string })[] {
  const keep = new Array<boolean>(lines.length).fill(false);
  lines.forEach((l, i) => {
    if (l.op !== "same") {
      for (let k = Math.max(0, i - context); k <= Math.min(lines.length - 1, i + context); k++) keep[k] = true;
    }
  });
  const out: (DiffLine | { op: "gap"; text: string })[] = [];
  let skipped = 0;
  lines.forEach((l, i) => {
    if (keep[i]) {
      if (skipped) out.push({ op: "gap", text: `… ${skipped} 行未变` });
      skipped = 0;
      out.push(l);
    } else skipped++;
  });
  if (skipped) out.push({ op: "gap", text: `… ${skipped} 行未变` });
  return out;
}
