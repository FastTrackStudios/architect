// /download and the landing page's Download buttons in a real browser
// (headless Firefox over WebDriver BiDi), with GitHub's API stubbed: the
// prerendered page's convention links, then what the page makes of the
// API's answer (src/lib/release-assets.ts, src/lib/release-check.ts):
// every asset there, the Mac build missing, the API down, rate limited.
// The stub is a preload script that replaces `fetch` for api.github.com,
// so nothing here reaches GitHub; the test also fails if the page asks for
// a release asset itself.
//
//   bun run build && env -u LD_LIBRARY_PATH node --test test/download.browser.mjs
//   (or `just site-download-test`; FIREFOX=… picks the browser,
//   SITE_OUTPUT=… the built site, SCREENSHOTS=<dir> saves /download in
//   each state at 1280 and 390 px)
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import {
	existsSync,
	mkdirSync,
	mkdtempSync,
	readFileSync,
	rmSync,
	writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const output = process.env.SITE_OUTPUT ?? join(site, ".output", "public");
const shots = process.env.SCREENSHOTS ?? "";
function firefox() {
	if (process.env.FIREFOX) return process.env.FIREFOX;
	try {
		return (
			execFileSync("sh", ["-c", "command -v firefox"]).toString().trim() || null
		);
	} catch {
		return null;
	}
}
function bun() {
	try {
		return (
			execFileSync("sh", ["-c", "command -v bun"]).toString().trim() || null
		);
	} catch {
		return null;
	}
}
const skip = !existsSync(join(output, "download.html"))
	? `no built site at ${output} (bun run build)`
	: !firefox()
		? "needs a Firefox (FIREFOX=…)"
		: !bun()
			? "needs bun (scripts/serve.ts)"
			: false;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const { tag } = JSON.parse(
	readFileSync(join(site, "src", "data", "release.json"), "utf8"),
);
const version = tag.replace(/^v/, "");
const download = (file) =>
	`https://github.com/FastTrackStudios/architect/releases/download/${tag}/${file}`;
const FILES = {
	linux: "Architect-linux-x86_64.AppImage",
	macos: "Architect-macos-arm64.dmg",
	node: `architect-node-${tag}-x86_64-linux.tar.gz`,
	web: `architect-web-${version}.tar.gz`,
	sums: "SHA256SUMS",
	sig: "SHA256SUMS.minisig",
};
const hex = (c) => c.repeat(64);
/** The API's release with every asset release.yml makes, sizes and digests. */
function fullRelease() {
	return {
		tag_name: tag,
		body: "notes",
		assets: [
			[FILES.linux, 73980408, "a"],
			[FILES.macos, 71820825, "b"],
			[FILES.node, 42046047, "c"],
			[`${FILES.node}.sha256`, 109, "0"],
			[FILES.web, 2131197, "d"],
			["latest-linux.json", 227, "1"],
			["latest-macos.json", 216, "2"],
			[FILES.sums, 561, "e"],
			[FILES.sig, 290, "f"],
		].map(([name, size, c]) => ({
			name,
			size,
			digest: `sha256:${hex(c)}`,
			state: "uploaded",
			browser_download_url: download(name),
		})),
	};
}

/** The files served by the bundle's own rules (scripts/serve.ts). */
async function serve() {
	const port = 19900 + Math.floor(Math.random() * 90);
	const proc = spawn(
		bun(),
		[join(site, "scripts", "serve.ts"), output, String(port)],
		{
			stdio: "ignore",
		},
	);
	const base = `http://127.0.0.1:${port}`;
	for (let i = 0; i < 100; i++) {
		try {
			await fetch(`${base}/`);
			return { base, stop: () => proc.kill() };
		} catch {
			await sleep(100);
		}
	}
	proc.kill();
	throw new Error("serve.ts did not start");
}

async function open() {
	const profile = mkdtempSync(join(tmpdir(), "architect-site-"));
	const port = 19000 + Math.floor(Math.random() * 900);
	const proc = spawn(
		firefox(),
		[
			"--headless",
			"--no-remote",
			"--profile",
			profile,
			"--remote-debugging-port",
			String(port),
		],
		{ stdio: "ignore" },
	);
	let ws;
	for (let i = 0; i < 80 && !ws; i++) {
		await sleep(250);
		try {
			const s = new WebSocket(`ws://127.0.0.1:${port}/session`);
			await new Promise((ok, no) => {
				s.onopen = ok;
				s.onerror = no;
			});
			ws = s;
		} catch {}
	}
	if (!ws) throw new Error("firefox did not start");
	let id = 0;
	const waiting = new Map();
	const requests = [];
	ws.onmessage = (e) => {
		const m = JSON.parse(e.data);
		if (m.method === "network.beforeRequestSent")
			requests.push(m.params.request.url);
		if (m.id && waiting.has(m.id)) {
			const w = waiting.get(m.id);
			waiting.delete(m.id);
			m.type === "error"
				? w.no(new Error(`${m.error}: ${m.message}`))
				: w.ok(m.result);
		}
	};
	const send = (method, params = {}) =>
		new Promise((ok, no) => {
			const i = ++id;
			waiting.set(i, { ok, no });
			ws.send(JSON.stringify({ id: i, method, params }));
		});
	await send("session.new", { capabilities: {} });
	await send("session.subscribe", { events: ["network.beforeRequestSent"] });
	const context = (await send("browsingContext.getTree", {})).contexts[0]
		.context;
	await send("browsingContext.navigate", {
		context,
		url: "about:blank",
		wait: "complete",
	});
	let preload = null;
	const tab = {
		requests,
		size: (width, height) =>
			send("browsingContext.setViewport", {
				context,
				viewport: { width, height },
			}),
		/**
		 * What `fetch` answers for api.github.com from now on: a JSON body
		 * and status, or `null` for a network failure. Counted in
		 * sessionStorage's twin, `window.__api` (this document's calls).
		 */
		async stub(answer) {
			if (preload)
				await send("script.removePreloadScript", { script: preload });
			const fn = `() => {
				const answer = ${JSON.stringify(answer)};
				const real = window.fetch.bind(window);
				window.__api = 0;
				window.fetch = async (input, init) => {
					const url = String(input instanceof Request ? input.url : input);
					if (!url.startsWith("https://api.github.com/")) return real(input, init);
					window.__api++;
					if (!answer) throw new TypeError("NetworkError when attempting to fetch resource.");
					return new Response(JSON.stringify(answer.body), {
						status: answer.status,
						headers: { "content-type": "application/json", ...answer.headers },
					});
				};
			}`;
			preload = (
				await send("script.addPreloadScript", { functionDeclaration: fn })
			).script;
		},
		go: (url) =>
			send("browsingContext.navigate", { context, url, wait: "complete" }),
		async eval(expression) {
			const r = await send("script.evaluate", {
				expression,
				target: { context },
				awaitPromise: true,
			});
			if (r.type === "exception") throw new Error(r.exceptionDetails.text);
			return r.result.value;
		},
		async until(expression, what, ms = 10000) {
			const started = Date.now();
			for (;;) {
				const v = await tab.eval(expression).catch(() => null);
				if (v) return v;
				if (Date.now() - started > ms)
					throw new Error(`waited ${ms} ms for ${what}`);
				await sleep(50);
			}
		},
		async shot(name) {
			if (!shots) return;
			mkdirSync(shots, { recursive: true });
			const r = await send("browsingContext.captureScreenshot", {
				context,
				origin: "document",
			});
			writeFileSync(join(shots, `${name}.png`), Buffer.from(r.data, "base64"));
		},
	};
	return {
		tab,
		close: () => {
			ws.close();
			proc.kill();
			try {
				rmSync(profile, { recursive: true, force: true });
			} catch {}
		},
	};
}

/** Each row's key, state and button, and the links under the list. */
const READ = `(() => {
	const rows = {};
	for (const li of document.querySelectorAll(".rows [data-asset]")) {
		rows[li.dataset.asset] = {
			state: li.dataset.state,
			href: li.querySelector("a.btn")?.getAttribute("href") ?? null,
			text: li.querySelector(".row-body")?.textContent ?? "",
		};
	}
	const links = {};
	for (const a of document.querySelectorAll(".page-links a[data-asset]"))
		links[a.dataset.asset] = a.getAttribute("href");
	return JSON.stringify({
		check: document.querySelector(".rows")?.dataset.releaseCheck,
		rows,
		links,
		api: window.__api,
		wide: document.documentElement.scrollWidth > window.innerWidth,
	});
})()`;
const read = async (tab) => JSON.parse(await tab.eval(READ));
/** Waits until the page has its answer (data-release-check off `pending`). */
const settle = (tab) =>
	tab.until(
		`document.querySelector(".rows")?.dataset.releaseCheck !== "pending"`,
		"the release check",
	);
/**
 * Opens /download with an empty cache: a first view (its lookup settled,
 * so nothing writes the cache late), the tab's cache cleared, then `url`.
 */
async function fresh(tab, base, url = "/download") {
	await tab.go(`${base}/download`);
	await settle(tab);
	await tab.eval("sessionStorage.clear(), true");
	await tab.go(`${base}${url}`);
}

test("/download: convention links, reconciled with the API", {
	skip,
	timeout: 120_000,
}, async () => {
	const server = await serve();
	const { tab, close } = await open();
	try {
		// No script: the HTML alone links every file by convention.
		const html = await (await fetch(`${server.base}/download`)).text();
		for (const file of Object.values(FILES))
			assert.ok(
				html.includes(`href="${download(file)}"`),
				`the HTML links ${file}`,
			);
		const landing = await (await fetch(`${server.base}/`)).text();
		assert.ok(landing.includes(`href="${download(FILES.linux)}"`));
		assert.ok(landing.includes(`href="${download(FILES.macos)}"`));

		const states = [
			["all", { status: 200, body: fullRelease() }],
			[
				"mac-missing",
				{
					status: 200,
					body: {
						...fullRelease(),
						assets: fullRelease().assets.filter((a) => !/macos/.test(a.name)),
					},
				},
			],
			["api-down", null],
		];
		for (const [name, answer] of states) {
			await tab.stub(answer);
			for (const [w, h] of [
				[1280, 900],
				[390, 844],
			]) {
				await tab.size(w, h);
				await fresh(tab, server.base);
				await settle(tab);
				const page = await read(tab);
				assert.equal(page.api, 1, `${name}: one API call per view`);
				assert.equal(page.wide, false, `${name} at ${w}px: no sideways scroll`);
				await tab.shot(`download-${name}-${w}`);

				if (name === "all") {
					assert.equal(page.check, "release");
					for (const key of ["linux", "macos", "node"]) {
						assert.equal(page.rows[key].state, "listed", key);
						assert.equal(page.rows[key].href, download(FILES[key]));
					}
					assert.match(page.rows.linux.text, /74\.0 MB/);
					assert.ok(page.rows.linux.text.includes(hex("a")), "its sha256");
					assert.deepEqual(page.links, {
						sums: download(FILES.sums),
						sig: download(FILES.sig),
						web: download(FILES.web),
					});
				} else if (name === "mac-missing") {
					assert.equal(page.rows.macos.state, "missing");
					assert.equal(page.rows.macos.href, null, "no dead link");
					assert.match(page.rows.macos.text, /Not available/);
					assert.equal(page.rows.linux.state, "listed");
				} else {
					assert.equal(page.check, "error");
					for (const key of ["linux", "macos", "node"]) {
						assert.equal(page.rows[key].state, "unchecked", key);
						assert.equal(page.rows[key].href, download(FILES[key]));
					}
					assert.equal(Object.keys(page.links).length, 3);
				}
			}
		}

		// The answer is kept for the tab: a second view asks nothing.
		await tab.stub({ status: 200, body: fullRelease() });
		await fresh(tab, server.base);
		await settle(tab);
		await tab.go(`${server.base}/download`);
		await settle(tab);
		assert.equal((await read(tab)).api, 0, "the second view is cached");

		// Rate limited: the links stay; the limit is kept, not asked again.
		await tab.stub({
			status: 403,
			body: { message: "API rate limit exceeded" },
			headers: { "x-ratelimit-remaining": "0" },
		});
		await fresh(tab, server.base);
		await settle(tab);
		let page = await read(tab);
		assert.equal(page.check, "rate-limited");
		assert.equal(page.rows.macos.state, "unchecked");
		assert.equal(page.rows.macos.href, download(FILES.macos));
		await tab.go(`${server.base}/download`);
		await settle(tab);
		page = await read(tab);
		assert.equal(page.api, 0, "a rate limit is not asked again");

		// The landing page: no Mac build, no Mac button; on a Mac the
		// one to /download stays.
		await tab.stub({
			status: 200,
			body: {
				...fullRelease(),
				assets: fullRelease().assets.filter((a) => !/macos/.test(a.name)),
			},
		});
		await tab.size(1280, 900);
		await fresh(tab, server.base, "/");
		await tab.until(
			`!document.querySelector(".hero-get .os-macos")`,
			"the Mac button to go",
		);
		assert.ok(
			await tab.eval(
				`document.documentElement.dataset.os = "macos", getComputedStyle(document.querySelector(".hero-get .os-any")).display !== "none"`,
			),
			"a Mac visitor still gets a Download button",
		);
		assert.ok(
			await tab.eval(
				`!!document.querySelector('.hero-get a.os-linux[href="${download(FILES.linux)}"]')`,
			),
		);

		// Never a release asset itself from script (its host sends no CORS).
		assert.deepEqual(
			tab.requests.filter((u) => u.includes("/releases/download/")),
			[],
		);
	} finally {
		close();
		server.stop();
	}
});
