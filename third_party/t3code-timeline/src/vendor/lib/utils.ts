// Vendored from T3 Code — apps/web/src/lib/utils.ts
// Copyright (c) 2026 T3 Tools Inc. — MIT (see ../../LICENSE.t3code).
// Upstream: https://github.com/pingdotgg/t3code (pin: UPSTREAM_COMMIT).
// Local edits are catalogued in ../../NOTICE.md; keep them minimal so a
// re-sync stays a copy rather than a merge.
import { CommandId, MessageId, ProjectId, ThreadId } from "@t3tools/contracts";
import { type CxOptions, cx } from "class-variance-authority";
import { twMerge } from "tailwind-merge";
import * as Random from "effect/Random";
import * as Effect from "effect/Effect";

export function cn(...inputs: CxOptions) {
  return twMerge(cx(inputs));
}

export function isMacPlatform(platform: string): boolean {
  return /mac|iphone|ipad|ipod/i.test(platform);
}

export function isWindowsPlatform(platform: string): boolean {
  return /^win(dows)?/i.test(platform);
}

export function isLinuxPlatform(platform: string): boolean {
  return /linux/i.test(platform);
}

export function randomUUID(): string {
  if (typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  return Effect.runSync(Random.nextUUIDv4);
}

export const newCommandId = (): CommandId => CommandId.makeUnsafe(randomUUID());

export const newProjectId = (): ProjectId => ProjectId.makeUnsafe(randomUUID());

export const newThreadId = (): ThreadId => ThreadId.makeUnsafe(randomUUID());

export const newMessageId = (): MessageId => MessageId.makeUnsafe(randomUUID());
