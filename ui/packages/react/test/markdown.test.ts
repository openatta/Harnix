import { describe, expect, it } from "vitest";
import { markdown } from "../src/markdown";

describe("markdown", () => {
  it("先转义再加标记：模型输出的 HTML 只是文字", () => {
    const html = markdown('<script>alert(1)</script> **粗** `x<y`');
    expect(html).not.toContain("<script>");
    expect(html).toContain("&lt;script&gt;");
    expect(html).toContain("<strong>粗</strong>");
    expect(html).toContain("<code>x&lt;y</code>");
  });

  it("围栏代码原样保留，不被行内规则改写", () => {
    const html = markdown("前\n```js\nconst a = **b**;\n```\n后");
    expect(html).toBe('<p>前</p><pre><code data-lang="js">const a = **b**;</code></pre><p>后</p>');
  });

  it("标题、列表、引用、表格、链接", () => {
    const html = markdown(
      "# 标题\n- a\n- b\n1. c\n> 引\n\n| h1 | h2 |\n|---|---|\n| x | y |\n\n[链](https://e.x/a)",
    );
    expect(html).toContain("<h1>标题</h1>");
    expect(html).toContain("<ul><li>a</li><li>b</li></ul><ol><li>c</li></ol>");
    expect(html).toContain("<blockquote>引</blockquote>");
    expect(html).toContain("<table><thead><tr><th>h1</th><th>h2</th></tr></thead><tbody><tr><td>x</td><td>y</td></tr></tbody></table>");
    expect(html).toContain('<a href="https://e.x/a" target="_blank" rel="noreferrer noopener">链</a>');
  });

  it("只有链接是 http(s) 时才成为链接", () => {
    expect(markdown("[x](javascript:alert(1))")).not.toContain("<a ");
  });

  it("没有分隔行的竖线行是普通文字", () => {
    expect(markdown("a | b\n下一行")).toBe("<p>a | b 下一行</p>");
  });
});
