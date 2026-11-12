// /download's links (src/lib/release-assets.ts, src/lib/release-check.ts):
// the convention, the API's answer reconciled with it, the tab's cache.
//   bun test test/
import { describe, expect, test } from "bun:test";

import {
	type ApiAsset,
	assetFiles,
	conventionAssets,
	formatSize,
	type Lookup,
	lookupFrom,
	reconcile,
	releaseApiUrl,
} from "../src/lib/release-assets";
import { CACHE_MS, lookupRelease } from "../src/lib/release-check";

const TAG = "v0.12.4";
const sha = (c: string) => c.repeat(64);

/** Every asset release.yml makes for TAG, as the API lists them. */
function full(): ApiAsset[] {
	return [
		{
			name: "Architect-linux-x86_64.AppImage",
			size: 73980408,
			digest: `sha256:${sha("a")}`,
			state: "uploaded",
		},
		{
			name: "Architect-macos-arm64.dmg",
			size: 71820825,
			digest: `sha256:${sha("b")}`,
			state: "uploaded",
		},
		{
			name: "architect-node-v0.12.4-x86_64-linux.tar.gz",
			size: 42046047,
			digest: `sha256:${sha("c")}`,
			state: "uploaded",
		},
		{
			name: "architect-node-v0.12.4-x86_64-linux.tar.gz.sha256",
			size: 109,
			state: "uploaded",
		},
		{
			name: "architect-web-0.12.4.tar.gz",
			size: 2131197,
			digest: `sha256:${sha("d")}`,
			state: "uploaded",
		},
		{ name: "latest-linux.json", size: 227, state: "uploaded" },
		{ name: "latest-macos.json", size: 216, state: "uploaded" },
		{
			name: "SHA256SUMS",
			size: 561,
			digest: `sha256:${sha("e")}`,
			state: "uploaded",
		},
		{
			name: "SHA256SUMS.minisig",
			size: 290,
			digest: `sha256:${sha("f")}`,
			state: "uploaded",
		},
	];
}
const release = (assets: ApiAsset[]): Lookup => ({
	kind: "release",
	release: { tag_name: TAG, assets },
});

describe("the convention", () => {
	test("names each asset as release.yml does", () => {
		expect(assetFiles(TAG)).toEqual({
			linux: "Architect-linux-x86_64.AppImage",
			macos: "Architect-macos-arm64.dmg",
			node: "architect-node-v0.12.4-x86_64-linux.tar.gz",
			web: "architect-web-0.12.4.tar.gz",
			sums: "SHA256SUMS",
			sig: "SHA256SUMS.minisig",
		});
	});
	test("links the tag's own downloads, none checked", () => {
		const a = conventionAssets(TAG);
		expect(a.macos.url).toBe(
			"https://github.com/FastTrackStudios/architect/releases/download/v0.12.4/Architect-macos-arm64.dmg",
		);
		expect(Object.values(a).every((x) => x.state === "unchecked")).toBe(true);
		expect(releaseApiUrl(TAG)).toBe(
			"https://api.github.com/repos/FastTrackStudios/architect/releases/tags/v0.12.4",
		);
	});
});

describe("reconcile", () => {
	test("all assets present: listed, with sizes and sha256", () => {
		const a = reconcile(TAG, release(full()));
		for (const x of Object.values(a)) expect(x.state).toBe("listed");
		expect(a.linux.size).toBe(73980408);
		expect(a.linux.sha256).toBe(sha("a"));
		expect(a.sig.sha256).toBe(sha("f"));
		// The links stay the convention's: never the API's asset URLs.
		expect(a.linux.url).toBe(conventionAssets(TAG).linux.url);
	});

	test("the Mac build missing: only it is missing", () => {
		const a = reconcile(
			TAG,
			release(full().filter((x) => !x.name.endsWith(".dmg"))),
		);
		expect(a.macos.state).toBe("missing");
		expect(a.macos.size).toBeNull();
		expect(a.linux.state).toBe("listed");
		expect(a.node.state).toBe("listed");
	});

	test("an upload still running is not there yet", () => {
		const assets = full().map((x) =>
			x.name === "SHA256SUMS" ? { ...x, state: "open" } : x,
		);
		expect(reconcile(TAG, release(assets)).sums.state).toBe("missing");
	});

	test("an asset with no digest is listed without a sha256", () => {
		const assets = full().map((x) =>
			x.name.endsWith(".AppImage") ? { ...x, digest: null } : x,
		);
		const a = reconcile(TAG, release(assets));
		expect(a.linux.state).toBe("listed");
		expect(a.linux.sha256).toBeNull();
	});

	test("an unexpected extra asset is ignored", () => {
		const a = reconcile(
			TAG,
			release([
				...full(),
				{ name: "Architect-windows-x86_64.msi", size: 1, state: "uploaded" },
			]),
		);
		expect(Object.keys(a).sort()).toEqual(
			["linux", "macos", "node", "web", "sums", "sig"].sort(),
		);
		expect(Object.values(a).every((x) => x.state === "listed")).toBe(true);
	});

	test("an API error keeps the convention's links", () => {
		expect(reconcile(TAG, { kind: "error", detail: "HTTP 500" })).toEqual(
			conventionAssets(TAG),
		);
	});

	test("a rate limit keeps the convention's links", () => {
		expect(reconcile(TAG, { kind: "rate-limited" })).toEqual(
			conventionAssets(TAG),
		);
	});
});

describe("lookupFrom", () => {
	const body = { tag_name: TAG, assets: full(), body: "notes", author: {} };
	test("a 200 for the tag is its release, trimmed", () => {
		const l = lookupFrom(TAG, 200, "59", body);
		expect(l.kind).toBe("release");
		if (l.kind === "release") {
			expect(l.release.assets.length).toBe(full().length);
			expect(Object.keys(l.release)).toEqual(["tag_name", "assets"]);
		}
	});
	test("a 403 with no requests left is the rate limit", () => {
		expect(lookupFrom(TAG, 403, "0", null)).toEqual({ kind: "rate-limited" });
		expect(lookupFrom(TAG, 429, null, null)).toEqual({ kind: "rate-limited" });
	});
	test("another 403, a 404 or a 5xx is an error", () => {
		expect(lookupFrom(TAG, 403, "41", null).kind).toBe("error");
		expect(lookupFrom(TAG, 404, "59", null).kind).toBe("error");
		expect(lookupFrom(TAG, 502, null, null).kind).toBe("error");
	});
	test("another tag's release, or a body with no assets, is an error", () => {
		expect(
			lookupFrom(TAG, 200, "59", { ...body, tag_name: "v0.12.3" }).kind,
		).toBe("error");
		expect(lookupFrom(TAG, 200, "59", { tag_name: TAG }).kind).toBe("error");
	});
});

describe("lookupRelease (the tab's cache)", () => {
	function memory() {
		const m = new Map<string, string>();
		return {
			getItem: (k: string) => m.get(k) ?? null,
			setItem: (k: string, v: string) => void m.set(k, v),
		};
	}
	function api(status: number, json: unknown, remaining = "59") {
		let calls = 0;
		const f = (async () => {
			calls++;
			return new Response(JSON.stringify(json), {
				status,
				headers: { "x-ratelimit-remaining": remaining },
			});
		}) as unknown as typeof fetch;
		return { f, calls: () => calls };
	}

	test("asks once, then answers from the cache until it is stale", async () => {
		const storage = memory();
		const { f, calls } = api(200, { tag_name: TAG, assets: full() });
		let now = 1_000;
		const deps = { fetch: f, storage, now: () => now };
		expect((await lookupRelease(TAG, deps)).kind).toBe("release");
		expect((await lookupRelease(TAG, deps)).kind).toBe("release");
		expect(calls()).toBe(1);
		now += CACHE_MS + 1;
		await lookupRelease(TAG, deps);
		expect(calls()).toBe(2);
	});

	test("keeps a rate limit, not an error", async () => {
		const limited = api(403, { message: "rate limit" }, "0");
		const storage = memory();
		const deps = { fetch: limited.f, storage, now: () => 0 };
		expect((await lookupRelease(TAG, deps)).kind).toBe("rate-limited");
		await lookupRelease(TAG, deps);
		expect(limited.calls()).toBe(1);

		const down = api(500, {});
		const deps2 = { fetch: down.f, storage: memory(), now: () => 0 };
		expect((await lookupRelease(TAG, deps2)).kind).toBe("error");
		await lookupRelease(TAG, deps2);
		expect(down.calls()).toBe(2);
	});

	test("a network failure is an error, and no storage still works", async () => {
		const f = (async () => {
			throw new TypeError("NetworkError");
		}) as unknown as typeof fetch;
		const l = await lookupRelease(TAG, {
			fetch: f,
			storage: null,
			now: () => 0,
		});
		expect(l.kind).toBe("error");
	});
});

test("formatSize", () => {
	expect(formatSize(561)).toBe("561 B");
	expect(formatSize(2131197)).toBe("2.13 MB");
	expect(formatSize(73980408)).toBe("74.0 MB");
});
