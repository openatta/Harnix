/**
 * 一个小的 markdown 渲染器：围栏代码、标题、列表、引用、分隔线、表格与常见行内格式。
 *
 * 移植自 旧界面 ui/runtime/markdown.js。刻意做小：不高亮、不排公式、不透传 HTML。
 * 加任何标记之前先把全部文本转义，所以模型输出 `<script>` 只会显示成文字——这是用
 * `dangerouslySetInnerHTML` 渲染它的前提。
 */

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export function markdown(source: string | null | undefined): string {
  const fences: string[] = [];
  const text = String(source || "").replace(/```(\w*)\n?([\s\S]*?)```/g, (_, lang: string, code: string) => {
    const cls = lang ? ` data-lang="${escapeHtml(lang)}"` : "";
    fences.push(`<pre><code${cls}>${escapeHtml(code.replace(/\n$/, ""))}</code></pre>`);
    return `\u0000${fences.length - 1}\u0000`;
  });

  const out: string[] = [];
  let list: "ul" | "ol" | null = null;
  let paragraph: string[] = [];
  let table: { pendingHeader?: string[]; open?: boolean } | null = null;

  const flushParagraph = () => {
    if (!paragraph.length) return;
    out.push(`<p>${inline(paragraph.join(" "))}</p>`);
    paragraph = [];
  };
  const closeList = () => {
    if (!list) return;
    out.push(`</${list}>`);
    list = null;
  };
  const closeTable = () => {
    if (!table) return;
    // 一直没等到分隔行的表头只是普通文字，丢掉它等于悄悄吃掉模型的一行。
    if (table.pendingHeader) out.push(`<p>${inline(table.pendingHeader.join(" | "))}</p>`);
    else out.push("</tbody></table>");
    table = null;
  };
  const closeAll = () => {
    flushParagraph();
    closeList();
    closeTable();
  };

  for (const raw of text.split("\n")) {
    const line = raw.replace(/\s+$/, "");

    if (!line.trim()) {
      closeAll();
      continue;
    }

    const fence = /^\u0000(\d+)\u0000$/.exec(line.trim());
    if (fence) {
      closeAll();
      out.push(fences[Number(fence[1])] ?? "");
      continue;
    }

    const heading = /^(#{1,4})\s+(.*)$/.exec(line);
    if (heading) {
      closeAll();
      const level = heading[1]!.length;
      out.push(`<h${level}>${inline(heading[2]!)}</h${level}>`);
      continue;
    }

    if (/^(-{3,}|\*{3,}|_{3,})$/.test(line.trim())) {
      closeAll();
      out.push("<hr>");
      continue;
    }

    // 表格：表头行之后跟一条分隔行。
    if (line.includes("|") && !table) {
      const cells = splitRow(line);
      if (cells) {
        flushParagraph();
        table = { pendingHeader: cells };
        continue;
      }
    } else if (table && table.pendingHeader) {
      if (/^\s*\|?[\s:|-]+\|?\s*$/.test(line) && line.includes("-")) {
        closeList();
        out.push(
          `<table><thead><tr>${table.pendingHeader.map((c) => `<th>${inline(c)}</th>`).join("")}</tr></thead><tbody>`,
        );
        table = { open: true };
      } else {
        // 终究不是表格：表头行是普通文字。
        paragraph.push(table.pendingHeader.join(" | "));
        table = null;
        paragraph.push(line);
      }
      continue;
    } else if (table && table.open) {
      const cells = splitRow(line);
      if (cells) {
        out.push(`<tr>${cells.map((c) => `<td>${inline(c)}</td>`).join("")}</tr>`);
        continue;
      }
      closeTable();
    }

    if (/^\s*[-*+]\s+/.test(line)) {
      flushParagraph();
      closeTable();
      if (list !== "ul") {
        closeList();
        out.push("<ul>");
        list = "ul";
      }
      out.push(`<li>${inline(line.replace(/^\s*[-*+]\s+/, ""))}</li>`);
      continue;
    }

    const ordered = /^\s*\d+[.)]\s+(.*)$/.exec(line);
    if (ordered) {
      flushParagraph();
      closeTable();
      if (list !== "ol") {
        closeList();
        out.push("<ol>");
        list = "ol";
      }
      out.push(`<li>${inline(ordered[1]!)}</li>`);
      continue;
    }

    if (/^>\s?/.test(line)) {
      closeAll();
      out.push(`<blockquote>${inline(line.replace(/^>\s?/, ""))}</blockquote>`);
      continue;
    }

    closeList();
    paragraph.push(line);
  }

  closeAll();
  return out.join("");
}

function splitRow(line: string): string[] | null {
  const trimmed = line.trim();
  if (!trimmed.includes("|")) return null;
  const body = trimmed.replace(/^\|/, "").replace(/\|$/, "");
  const cells = body.split("|").map((c) => c.trim());
  return cells.length > 1 ? cells : null;
}

function inline(text: string): string {
  return escapeHtml(text)
    .replace(/`([^`]+)`/g, (_, code: string) => `<code>${code}</code>`)
    .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[\s(（])\*([^*\n]+)\*/g, "$1<em>$2</em>")
    .replace(/\[([^\]]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer noopener">$1</a>');
}
