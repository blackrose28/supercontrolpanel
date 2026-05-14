import { expect, test } from "@playwright/test";
import { buildEnrollmentFormDefaultsFromCcConfig } from "../src/ccEnrollmentFormDefaults";

test("buildEnrollmentFormDefaultsFromCcConfig trims server URL and device name", () => {
  expect(
    buildEnrollmentFormDefaultsFromCcConfig({
      ccServerUrl: "  http://localhost:3000  ",
      ccDeviceName: "  Desk  ",
    }),
  ).toEqual({
    serverUrl: "http://localhost:3000",
    deviceName: "Desk",
  });
});

test("buildEnrollmentFormDefaultsFromCcConfig handles missing device name", () => {
  expect(
    buildEnrollmentFormDefaultsFromCcConfig({
      ccServerUrl: "",
      ccDeviceName: undefined,
    }),
  ).toEqual({
    serverUrl: "",
    deviceName: "",
  });
});
