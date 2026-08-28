import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { opCall } from "./generated/api";
import type { OperationId } from "./generated/ops";
import { CommandPalette } from "./command-palette";
import { WaveformTimeline, totalSamples } from "./waveform";
import { isLiveState, nextSelectedId, pollIntervalMs, sortedByNewest } from "./live";
import "./style.css";

export interface Run { data: number; count: number }
export interface Capture { id: number; seq: number; sample_period_s: number; trigger_sample: number; reference_sample: number; channels_acquired: number; runs: Run[] }
interface DeviceStatus { state: string; backend: string; fpga: string; fpga_image_id: number | null; usb_error_count: number; needs_replug?: boolean; error?: string | null }
interface AcquisitionStatus { state: string; samples: number; acq_count: number; recurring: boolean; buffer_fill_pct: number }
interface SampleSettings { mode: string; rate_index: number; rate_hz: number; compression: boolean }
interface CaptureEnvelope { capture: Capture }
const channels = Array.from({ length: 34 }, (_, index) => index);

function App() {
  const [device, setDevice] = useState<DeviceStatus | null>(null);
  const [acquisition, setAcquisition] = useState<AcquisitionStatus | null>(null);
  const [sample, setSample] = useState<SampleSettings | null>(null);
  const [captures, setCaptures] = useState<Capture[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [importOpen, setImportOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [following, setFollowing] = useState(true);
  const followingRef = useRef(true);
  useEffect(() => { followingRef.current = following; }, [following]);
  const [cursorSample, setCursorSample] = useState<number | null>(null);
  // The cursor's sample index is capture-relative, so drop it when the shown
  // capture changes (e.g. auto-following a new one).
  useEffect(() => { setCursorSample(null); }, [selectedId]);
  const [importPath, setImportPath] = useState("/usr/local/share/logicport/examples/Quickstart.LPF");
  const refresh = useCallback(async () => {
    const [deviceResult, acquisitionResult, sampleResult, captureResult] = await Promise.all([
      opCall<DeviceStatus>("device.status"), opCall<AcquisitionStatus>("acq.status"),
      opCall<{ sample: SampleSettings }>("sample.get"), opCall<{ captures: CaptureEnvelope[] }>("capture.list", { limit: 20 }),
    ]);
    const nextCaptures = captureResult.captures.map((entry) => entry.capture);
    setDevice(deviceResult); setAcquisition(acquisitionResult); setSample(sampleResult.sample); setCaptures(nextCaptures);
    setSelectedId((current) => nextSelectedId(nextCaptures, current, followingRef.current));
  }, []);
  useEffect(() => { void refresh().catch((cause: unknown) => setError(message(cause))); }, [refresh]);
  const live = isLiveState(acquisition);
  const period = pollIntervalMs(live);
  // Always poll so fresh captures surface on their own; briskly while a capture
  // is filling, calmly when idle. No manual refresh is ever needed.
  useEffect(() => {
    const timer = window.setInterval(() => {
      void refresh().catch((cause: unknown) => setError(message(cause)));
    }, period);
    return () => window.clearInterval(timer);
  }, [period, refresh]);
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") { event.preventDefault(); setPaletteOpen(true); }
      if (event.key === "Escape") setPaletteOpen(false);
    };
    window.addEventListener("keydown", keydown);
    return () => window.removeEventListener("keydown", keydown);
  }, []);
  const invoke = useCallback(async (id: OperationId, params: unknown = {}) => {
    setBusy(true); setError(null);
    try { await opCall(id, params); await refresh(); } catch (cause: unknown) { setError(message(cause)); } finally { setBusy(false); }
  }, [refresh]);
  const ordered = useMemo(() => sortedByNewest(captures), [captures]);
  const selected = useMemo(() => captures.find((capture) => capture.id === selectedId) ?? ordered[0] ?? null, [captures, ordered, selectedId]);
  return <div className="app-shell">
    <header className="topbar"><div className="brand"><span className="brand-mark">MP</span><strong>MagicPort</strong></div><div className="transport-controls"><button className="primary" disabled={busy} onClick={() => void invoke("acq.single")}>▶ Capture</button><button disabled={busy} onClick={() => void invoke("acq.halt")}>■ Stop</button><button disabled={busy} onClick={() => void invoke("acq.trigger_immediate")}>Trigger now</button><button disabled={busy} onClick={() => setImportOpen(true)}>Import LPF</button><button disabled={busy} onClick={() => setPaletteOpen(true)}>Operations <kbd>Ctrl K</kbd></button></div><div className="device-pill" data-state={device?.state ?? "offline"}><span className="status-dot" /><span>{device?.backend ?? "connecting"}</span><small>{device == null || device.fpga_image_id == null ? "" : `FPGA ${hex(device.fpga_image_id)}`}</small></div></header>
    <aside className="left-panel"><section><h2>Acquisition</h2><label>Mode<select value={sample?.mode ?? "timing"} disabled={busy} onChange={(event) => void invoke("sample.apply", { sample: { ...sample, mode: event.target.value } })}><option value="timing">Timing</option><option value="state">State</option></select></label><label>Sample rate<select value={sample?.rate_index ?? 0} disabled={busy} onChange={(event) => void invoke("sample.apply", { sample: { ...sample, rate_index: Number(event.target.value) } })}>{rateLabels.map((label, index) => <option key={label} value={index}>{label}</option>)}</select></label><label className="check"><input type="checkbox" checked={sample?.compression ?? false} disabled={busy} onChange={(event) => void invoke("sample.apply", { sample: { ...sample, compression: event.target.checked } })} />Compression</label></section><section><h2>Channels</h2><div className="channel-list">{channels.map((channel) => <div className="channel-row" key={channel}><span className={channel < 32 ? `swatch c${channel % 8}` : "swatch clk"} /><b>{channelLabel(channel)}</b><span>{selected === null ? "—" : channelValue(selected, channel, cursorSample)}</span></div>)}</div></section></aside>
    <main className="workspace"><div className="timeline-toolbar"><div><strong>{selected === null ? "No capture" : `Capture ${selected.id}`}</strong><span>{selected === null ? "Run an acquisition to begin" : `${expandedLength(selected).toLocaleString()} samples · ${formatPeriod(selected.sample_period_s)}`}</span>{selected !== null && cursorSample !== null && <button className="cursor-chip" title="Clear cursor" onClick={() => setCursorSample(null)}>◆ {cursorSample.toLocaleString()} · {formatTime(cursorSample * selected.sample_period_s)} ✕</button>}</div><div className="buffer"><span style={{ width: `${acquisition?.buffer_fill_pct ?? 0}%` }} /></div><output>{live && <span className="live-dot" aria-label="live" />}{acquisition?.state ?? "idle"}</output></div>{selected === null ? <div className="empty-state"><div className="empty-icon">⌁</div><h1>Ready to capture</h1><p>Connect signals D0–D31 and start a single acquisition.</p></div> : <WaveformTimeline capture={selected} channels={channels} cursorSample={cursorSample} onCursor={setCursorSample} />}{error !== null && <div className="error-banner" role="alert">{error}<button onClick={() => setError(null)}>×</button></div>}</main>
    <aside className="right-panel"><section><h2>Session</h2><dl><dt>Status</dt><dd>{acquisition?.state ?? "—"}</dd><dt>Captures</dt><dd>{acquisition?.acq_count ?? 0}</dd><dt>USB errors</dt><dd>{device?.usb_error_count ?? 0}</dd></dl></section><section className="history"><h2>Capture history{!following && captures.length > 0 && <button className="follow-latest" onClick={() => { setFollowing(true); setSelectedId(ordered[0]?.id ?? null); }}>Jump to latest</button>}</h2>{captures.length === 0 ? <p className="muted">No captures yet</p> : ordered.map((capture) => <button className={capture.id === selected?.id ? "selected" : ""} key={capture.id} onClick={() => { setFollowing(false); setSelectedId(capture.id); }}><span>Capture {capture.id}</span><small>{expandedLength(capture).toLocaleString()} samples</small></button>)}</section></aside>
    {importOpen && <div className="modal-backdrop" role="presentation"><form className="modal" aria-label="Import LPF project" onSubmit={(event) => { event.preventDefault(); void invoke("project.import_lpf", { path: importPath }).then(() => setImportOpen(false)); }}><h1>Import LPF project</h1><label>LPF path<input autoFocus value={importPath} onChange={(event) => setImportPath(event.target.value)} /></label><div className="modal-actions"><button type="button" disabled={busy} onClick={() => setImportOpen(false)}>Cancel</button><button className="primary" type="submit" disabled={busy || importPath.trim() === ""}>Import</button></div></form></div>}
    <CommandPalette open={paletteOpen} busy={busy} onClose={() => setPaletteOpen(false)} onInvoke={invoke} />
  </div>;
}
const rateLabels = ["500 MHz", "200 MHz", "100 MHz", "50 MHz", "20 MHz", "10 MHz", "5 MHz", "2 MHz", "1 MHz", "500 kHz", "200 kHz", "100 kHz", "50 kHz", "20 kHz", "10 kHz", "5 kHz", "2 kHz", "1 kHz"];
function expandedLength(capture: Capture): number { return capture.runs.reduce((sum, run) => sum + run.count, 0); }
// JS bitwise `&` truncates to 32 bits, so it cannot test channels 32/33 (the
// clocks). Extract the bit by division instead; `data` (a double) holds all 34
// channel bits exactly.
function channelBit(data: number, channel: number): number { return Math.floor(data / 2 ** channel) % 2; }
function channelLabel(channel: number): string { return channel < 32 ? `D${channel}` : channel === 32 ? "CLK1" : "CLK2"; }
function sampleValueAt(capture: Capture, sample: number): number { let acc = 0; for (const run of capture.runs) { acc += run.count; if (sample < acc) return run.data; } return capture.runs.at(-1)?.data ?? 0; }
function channelValue(capture: Capture, channel: number, sample: number | null): string { const data = sample === null ? (capture.runs.at(-1)?.data ?? 0) : sampleValueAt(capture, sample); return channelBit(data, channel) === 0 ? "0" : "1"; }
function formatTime(seconds: number): string { const s = Math.abs(seconds); if (s < 1e-6) return `${(seconds * 1e9).toFixed(1)} ns`; if (s < 1e-3) return `${(seconds * 1e6).toFixed(2)} µs`; if (s < 1) return `${(seconds * 1e3).toFixed(3)} ms`; return `${seconds.toFixed(4)} s`; }
function formatPeriod(seconds: number): string { if (seconds < 1e-9) return `${(seconds * 1e12).toFixed(1)} ps/sample`; if (seconds < 1e-6) return `${(seconds * 1e9).toFixed(1)} ns/sample`; if (seconds < 1e-3) return `${(seconds * 1e6).toFixed(1)} µs/sample`; return `${(seconds * 1e3).toFixed(1)} ms/sample`; }
function hex(value: number | null | undefined): string { return value == null ? "?" : `0x${value.toString(16).padStart(2, "0")}`; }
function message(cause: unknown): string { return cause instanceof Error ? cause.message : String(cause); }
const root = document.getElementById("root"); if (root === null) throw new Error("missing #root"); createRoot(root).render(<StrictMode><App /></StrictMode>);
