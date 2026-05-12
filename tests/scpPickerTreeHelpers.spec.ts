import { expect, test } from "@playwright/test";
import {
  isStrictRemoteChildPath,
  uniqueDirectoryChildPathsForTree,
} from "../src/scpPickerTreeHelpers";

function normalizeRemotePath(path: string): string {
  const trimmed = path.trim();
  if (!trimmed) {
    return "";
  }
  if (trimmed === "/") {
    return "/";
  }
  return trimmed.replace(/\/+$/, "");
}

test("isStrictRemoteChildPath rejects parent and non-descendants", () => {
  expect(isStrictRemoteChildPath("/home/u", "/home/u")).toBe(false);
  expect(isStrictRemoteChildPath("/home/u", "/home/v")).toBe(false);
  expect(isStrictRemoteChildPath("/home/u", "/home/ux")).toBe(false);
  expect(isStrictRemoteChildPath("/", "/")).toBe(false);
  expect(isStrictRemoteChildPath("/", "/tmp")).toBe(true);
});

test("uniqueDirectoryChildPathsForTree drops self-path and dedupes", () => {
  const parent = "/data/proj";
  const paths = uniqueDirectoryChildPathsForTree(
    parent,
    [
      { path: "/data/proj", kind: "directory" },
      { path: "/data/proj/sub", kind: "directory" },
      { path: "/data/proj/sub", kind: "directory" },
      { path: "/data/other", kind: "directory" },
      { path: "/data/proj/file.txt", kind: "file" },
    ],
    normalizeRemotePath,
  );
  expect(paths).toEqual(["/data/proj/sub"]);
});
