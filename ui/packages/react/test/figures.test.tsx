/** The figures: a table sorted by the header clicked (numbers largest first, words A to Z, again to turn it), a row of bars. */

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { Bars, FigureTable, type FigureColumn } from "../src/index";

afterEach(cleanup);

interface Row {
  name: string;
  n: number;
}

const columns: FigureColumn<Row>[] = [
  { id: "name", label: "Name", value: (r) => r.name },
  { id: "n", label: "N", numeric: true, value: (r) => r.n, render: (r) => `${r.n}!` },
];
const rows: Row[] = [
  { name: "b", n: 1 },
  { name: "a", n: 3 },
  { name: "c", n: 2 },
];
const order = (el: HTMLElement) => [...el.querySelectorAll("tbody tr")].map((tr) => tr.getAttribute("data-key"));

describe("FigureTable", () => {
  it("sorts by the first numeric column, largest first; a header sorts by its column, again turns it", () => {
    const view = render(<FigureTable rows={rows} columns={columns} rowKey={(r) => r.name} dataKey={(r) => r.name} testId="t" />);
    const table = view.getByTestId("t");
    expect(order(table)).toEqual(["a", "c", "b"]);
    expect(table.querySelector('td[data-column="n"]')?.textContent).toBe("3!");
    expect(table.querySelector('th[data-column="n"]')?.getAttribute("aria-sort")).toBe("descending");
    fireEvent.click(table.querySelector('th[data-column="name"] button')!);
    expect(order(table)).toEqual(["a", "b", "c"]);
    fireEvent.click(table.querySelector('th[data-column="name"] button')!);
    expect(order(table)).toEqual(["c", "b", "a"]);
    expect(table.querySelector('th[data-column="n"]')?.getAttribute("aria-sort")).toBeNull();
  });
});

describe("Bars", () => {
  it("a bar per value, the largest full height, a zero only a baseline, each with its tooltip", () => {
    const view = render(
      <Bars
        testId="b"
        label="per day"
        bars={[
          { key: "d1", label: "01", value: 0, title: "none" },
          { key: "d2", label: "02", value: 50, title: "half" },
          { key: "d3", label: "03", value: 100, title: "all" },
        ]}
      />,
    );
    const cols = [...view.getByTestId("b").querySelectorAll<HTMLElement>(".col")];
    expect(cols.map((c) => c.title)).toEqual(["none", "half", "all"]);
    expect(cols.map((c) => c.querySelector("i")!.style.height)).toEqual(["", "50%", "100%"]);
    expect(cols[0]!.querySelector("i")!.className).toBe("zero");
  });
});
