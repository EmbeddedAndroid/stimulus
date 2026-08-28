import { useEffect, useRef, type MouseEvent } from "react";
import type { Capture } from "./main";
interface Props {
  capture: Capture;
  channels: number[];
  cursorSample?: number | null;
  onCursor?: (sample: number) => void;
}

/// Total expanded sample count of a capture.
export function totalSamples(capture: Capture): number {
  return capture.runs.reduce((sum, run) => sum + run.count, 0);
}
// LA1034 probe wire colors (resistor color code: D0 black, D1 brown, D2 red, D3
// orange, D4 yellow, D5 green, D6 blue, D7 violet), repeating every 8 channels.
// Brightened for trace visibility on the dark canvas; the black wire (D0) is
// drawn as light gray since pure black cannot render against the dark ground.
export const WIRE_COLORS: readonly [number, number, number][] = [
  [0.80, 0.82, 0.85], [0.69, 0.42, 0.18], [1.00, 0.30, 0.30], [1.00, 0.62, 0.24],
  [0.97, 0.85, 0.29], [0.31, 0.88, 0.42], [0.35, 0.66, 1.00], [0.69, 0.42, 1.00],
];
export function wireColor(channel: number): readonly [number, number, number] {
  return WIRE_COLORS[((channel % 8) + 8) % 8] ?? WIRE_COLORS[0]!;
}
export function WaveformTimeline({ capture, channels, cursorSample, onCursor }: Props) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const element = canvas.current;
    if (element === null) return;
    const render = () => draw(element, capture, channels, cursorSample ?? null);
    const observer = new ResizeObserver(render);
    observer.observe(element);
    render();
    return () => observer.disconnect();
  }, [capture, channels, cursorSample]);
  // Click anywhere on the timeline to drop the cursor at that sample; the
  // channel panel then reads each channel's value at the cursor.
  const place = (event: MouseEvent<HTMLCanvasElement>) => {
    if (onCursor === undefined) return;
    const total = totalSamples(capture);
    if (total <= 0) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const fraction = Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width));
    onCursor(Math.min(total - 1, Math.round(fraction * total)));
  };
  return (
    <canvas
      ref={canvas}
      className="waveform"
      onClick={place}
      aria-label={`Waveform for capture ${capture.id}`}
    />
  );
}
// One channel's line segments, placed at `row` of `rows` total rows.
function buildChannelVertices(capture: Capture, channel: number, row: number, rows: number): Float32Array {
  const total = capture.runs.reduce((sum, run) => sum + run.count, 0);
  if (total <= 0 || rows <= 0) return new Float32Array();
  const vertices: number[] = []; let sample = 0; let previous: number | null = null;
  for (const run of capture.runs) {
    const level = Math.floor(run.data / 2 ** channel) % 2;
    const x0 = sample / total * 2 - 1; const x1 = (sample + run.count) / total * 2 - 1;
    const y = 1 - ((row + 0.72 - level * 0.44) / rows) * 2;
    if (previous !== null && previous !== level) { const oldY = 1 - ((row + 0.72 - previous * 0.44) / rows) * 2; vertices.push(x0, oldY, x0, y); }
    vertices.push(x0, y, x1, y); previous = level; sample += run.count;
  }
  return new Float32Array(vertices);
}
// All channels in one array (single color) — retained for the unit test.
export function buildWaveVertices(capture: Capture, channels: number[]): Float32Array { const total = capture.runs.reduce((sum, run) => sum + run.count, 0); if (total <= 0 || channels.length === 0) return new Float32Array(); const vertices: number[] = []; for (let row = 0; row < channels.length; row += 1) { const channel = channels[row] ?? 0; let sample = 0; let previous: number | null = null; for (const run of capture.runs) { const level = Math.floor(run.data / 2 ** channel) % 2; const x0 = sample / total * 2 - 1; const x1 = (sample + run.count) / total * 2 - 1; const y = 1 - ((row + 0.72 - level * 0.44) / channels.length) * 2; if (previous !== null && previous !== level) { const oldY = 1 - ((row + 0.72 - previous * 0.44) / channels.length) * 2; vertices.push(x0, oldY, x0, y); } vertices.push(x0, y, x1, y); previous = level; sample += run.count; } } return new Float32Array(vertices); }

// Faint instrument grid: one horizontal guide per channel row plus evenly
// spaced vertical time ticks. Returned as GL_LINES vertex pairs in clip space.
function buildGrid(rows: number, ticks: number): Float32Array {
  const vertices: number[] = [];
  for (let row = 1; row < rows; row += 1) {
    const y = 1 - (row / rows) * 2;
    vertices.push(-1, y, 1, y);
  }
  for (let tick = 1; tick < ticks; tick += 1) {
    const x = (tick / ticks) * 2 - 1;
    vertices.push(x, -1, x, 1);
  }
  return new Float32Array(vertices);
}

type RGBA = readonly [number, number, number, number];

function drawLines(
  gl: WebGL2RenderingContext,
  position: number,
  colorLoc: WebGLUniformLocation | null,
  offsetLoc: WebGLUniformLocation | null,
  vertices: Float32Array,
  color: RGBA,
  offsetY: number,
) {
  if (vertices.length === 0) return;
  const buffer = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);
  gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
  gl.uniform2f(offsetLoc, 0, offsetY);
  gl.uniform4f(colorLoc, color[0], color[1], color[2], color[3]);
  gl.drawArrays(gl.LINES, 0, vertices.length / 2);
  gl.deleteBuffer(buffer);
}

function draw(canvas: HTMLCanvasElement, capture: Capture, channels: number[], cursorSample: number | null) {
  const ratio = window.devicePixelRatio || 1;
  const width = Math.max(1, Math.floor(canvas.clientWidth * ratio));
  const height = Math.max(1, Math.floor(canvas.clientHeight * ratio));
  if (canvas.width !== width || canvas.height !== height) { canvas.width = width; canvas.height = height; }
  const gl = canvas.getContext("webgl2", { antialias: true, premultipliedAlpha: false });
  if (gl === null) { drawFallback(canvas, capture, channels); return; }
  gl.viewport(0, 0, width, height);
  gl.clearColor(0.024, 0.043, 0.062, 1); gl.clear(gl.COLOR_BUFFER_BIT);
  const program = createProgram(gl); if (program === null) return;
  gl.useProgram(program);
  const position = gl.getAttribLocation(program, "position");
  const colorLoc = gl.getUniformLocation(program, "color");
  const offsetLoc = gl.getUniformLocation(program, "uOffset");
  gl.enableVertexAttribArray(position);
  gl.enable(gl.BLEND);

  // Instrument grid, drawn first and dim so traces sit on top of it.
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  drawLines(gl, position, colorLoc, offsetLoc, buildGrid(channels.length, 10), [0.16, 0.24, 0.33, 0.55], 0);

  const glowStep = (2 / height); // one device pixel in clip space
  const channelGeometry = channels.map((channel, row) => ({
    channel,
    vertices: buildChannelVertices(capture, channel, row, channels.length),
  }));

  // Soft glow: the same trace stacked at small vertical offsets, added together.
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE);
  for (const { channel, vertices } of channelGeometry) {
    const [r, g, b] = wireColor(channel);
    for (const offset of [-2.4, -1.1, 1.1, 2.4]) {
      drawLines(gl, position, colorLoc, offsetLoc, vertices, [r, g, b, 0.12], offset * glowStep);
    }
  }

  // Crisp core traces on top.
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  for (const { channel, vertices } of channelGeometry) {
    const [r, g, b] = wireColor(channel);
    drawLines(gl, position, colorLoc, offsetLoc, vertices, [Math.min(1, r * 1.12 + 0.06), Math.min(1, g * 1.12 + 0.06), Math.min(1, b * 1.12 + 0.06), 1], 0);
  }

  drawMarker(gl, position, colorLoc, offsetLoc, capture, glowStep);

  // Movable measurement cursor (accent cyan), distinct from the fixed red
  // trigger marker. Placed by clicking the timeline.
  if (cursorSample !== null) {
    const total = totalSamples(capture);
    if (total > 0) {
      const x = (cursorSample / total) * 2 - 1;
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE);
      for (const offset of [-1.5, 1.5]) {
        drawLines(gl, position, colorLoc, offsetLoc, new Float32Array([x + offset * glowStep, -1, x + offset * glowStep, 1]), [0.24, 0.84, 1, 0.35], 0);
      }
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      drawLines(gl, position, colorLoc, offsetLoc, new Float32Array([x, -1, x, 1]), [0.36, 0.9, 1, 0.95], 0);
    }
  }
  gl.deleteProgram(program);
}

function drawMarker(gl: WebGL2RenderingContext, position: number, colorLoc: WebGLUniformLocation | null, offsetLoc: WebGLUniformLocation | null, capture: Capture, step: number) {
  const total = capture.runs.reduce((sum, run) => sum + run.count, 0);
  if (total <= 0) return;
  const x = capture.trigger_sample / total * 2 - 1;
  const line = new Float32Array([x, -1, x, 1]);
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE);
  for (const offset of [-1, 1]) {
    const marker = new Float32Array([x + offset * step, -1, x + offset * step, 1]);
    drawLines(gl, position, colorLoc, offsetLoc, marker, [1, 0.45, 0.34, 0.25], 0);
  }
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  drawLines(gl, position, colorLoc, offsetLoc, line, [1, 0.5, 0.36, 0.9], 0);
}

function createProgram(gl: WebGL2RenderingContext): WebGLProgram | null { const vertex = shader(gl, gl.VERTEX_SHADER, "#version 300 es\nin vec2 position; uniform vec2 uOffset; void main(){gl_Position=vec4(position+uOffset,0.,1.);}"); const fragment = shader(gl, gl.FRAGMENT_SHADER, "#version 300 es\nprecision mediump float; uniform vec4 color; out vec4 outColor; void main(){outColor=color;}"); if (vertex === null || fragment === null) return null; const program = gl.createProgram(); if (program === null) return null; gl.attachShader(program, vertex); gl.attachShader(program, fragment); gl.linkProgram(program); gl.deleteShader(vertex); gl.deleteShader(fragment); return gl.getProgramParameter(program, gl.LINK_STATUS) === true ? program : null; }
function shader(gl: WebGL2RenderingContext, kind: number, source: string): WebGLShader | null { const value = gl.createShader(kind); if (value === null) return null; gl.shaderSource(value, source); gl.compileShader(value); return gl.getShaderParameter(value, gl.COMPILE_STATUS) === true ? value : null; }
function drawFallback(canvas: HTMLCanvasElement, capture: Capture, channels: number[]) {
  const context = canvas.getContext("2d"); if (context === null) return;
  context.fillStyle = "#070d15"; context.fillRect(0, 0, canvas.width, canvas.height);
  context.strokeStyle = "rgba(40,60,84,.5)"; context.lineWidth = 1;
  for (let row = 1; row < channels.length; row += 1) { const y = row / channels.length * canvas.height; context.beginPath(); context.moveTo(0, y); context.lineTo(canvas.width, y); context.stroke(); }
  for (let row = 0; row < channels.length; row += 1) {
    const channel = channels[row] ?? 0;
    const points = buildChannelVertices(capture, channel, row, channels.length);
    const [r, g, b] = wireColor(channel);
    context.strokeStyle = `rgb(${Math.round(r * 255)},${Math.round(g * 255)},${Math.round(b * 255)})`;
    context.lineWidth = 1.6; context.shadowColor = context.strokeStyle; context.shadowBlur = 6;
    context.beginPath();
    for (let index = 0; index < points.length; index += 4) { context.moveTo(((points[index] ?? 0) + 1) * canvas.width / 2, (1 - (points[index + 1] ?? 0)) * canvas.height / 2); context.lineTo(((points[index + 2] ?? 0) + 1) * canvas.width / 2, (1 - (points[index + 3] ?? 0)) * canvas.height / 2); }
    context.stroke();
  }
  context.shadowBlur = 0;
}
