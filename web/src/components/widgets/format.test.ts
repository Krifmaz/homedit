import { formatBytes, formatStatValue } from "./format";

describe("formatBytes", () => {
  it("formats bytes with SI units", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(1500)).toBe("1.5 kB");
    expect(formatBytes(6.2e9)).toBe("6.2 GB");
    expect(formatBytes(16e9)).toBe("16 GB");
    expect(formatBytes(1.1e12)).toBe("1.1 TB");
  });

  it("moves up a unit when rounding reaches 1000", () => {
    expect(formatBytes(999_960)).toBe("1 MB");
  });

  it("honours the digits argument", () => {
    expect(formatBytes(1_234_567, 2)).toBe("1.23 MB");
  });

  it("rejects invalid input", () => {
    expect(formatBytes(-1)).toBe("–");
    expect(formatBytes(Number.NaN)).toBe("–");
    expect(formatBytes(Infinity)).toBe("–");
  });
});

describe("formatStatValue", () => {
  it("keeps strings and formats numbers", () => {
    expect(formatStatValue("ok")).toBe("ok");
    expect(formatStatValue(4)).toBe("4");
    expect(formatStatValue(Number.NaN)).toBe("–");
  });
});
