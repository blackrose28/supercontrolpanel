import { expect, test } from "@playwright/test";
import { withTimeout } from "../src/scpPickerAsync";

test("withTimeout rejects when promise hangs", async () => {
  const never = new Promise<string>(() => {});
  await expect(
    withTimeout(never, 20, "Loading remote folder listing"),
  ).rejects.toThrow("Loading remote folder listing timed out");
});

test("withTimeout resolves when promise completes", async () => {
  await expect(withTimeout(Promise.resolve("ok"), 100, "any")).resolves.toBe(
    "ok",
  );
});
