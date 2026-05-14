export type CcEnrollmentFormDefaultsInput = {
  ccServerUrl: string;
  ccDeviceName?: string | null;
};

export function buildEnrollmentFormDefaultsFromCcConfig(
  input: CcEnrollmentFormDefaultsInput,
): { serverUrl: string; deviceName: string } {
  return {
    serverUrl: input.ccServerUrl.trim(),
    deviceName: (input.ccDeviceName ?? "").trim(),
  };
}
