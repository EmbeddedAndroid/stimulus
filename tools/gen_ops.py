#!/usr/bin/env python3
"""Generate the canonical 465-operation catalog and derived JSON/docs."""
from __future__ import annotations
import difflib, json, pathlib, re, sys

ROOT=pathlib.Path(__file__).resolve().parents[1]
ops=[]
def add(area, ids, mutating=()):
    mutable=set(mutating)
    for op_id in ids.split():
        ops.append({"id":op_id,"area":area,"title":op_id.replace("."," ").replace("_"," ").title(),"mutating":op_id in mutable or op_id.endswith((".set",".ok",".apply",".toggle")),"origin":"","ui":["Palette"],"shortcut":None,"rest":{"method":"POST","path":"/api/ops/"+op_id},"mcp":{"tool":"op_call","op":op_id},"truth":"Invariant","params":{"type":"object","additionalProperties":True},"result":{}})

def expanded(prefix, names): return " ".join(prefix+name for name in names.split())

add("device","device.enumerate device.connect device.disconnect device.demo.enter device.demo.exit device.status device.diagnose device.selftest device.usb_error_count.get device.usb_error_count.reset device.regs.read device.regs.write device.pins.read device.fpga.configure device.transcript.start device.transcript.stop device.wire_status.read device.freq_counter.read", "device.connect device.disconnect device.demo.enter device.demo.exit device.selftest device.usb_error_count.reset device.regs.write device.fpga.configure device.transcript.start device.transcript.stop".split())
add("sample","sample.get sample.mode.set sample.rate.set sample.rate.step_up sample.rate.step_down sample.rate.units.set sample.state.clock.set sample.state.edge.set sample.state.window.set sample.state.qualifier.enable sample.state.qualifier.polarity sample.state.declared_rate.set sample.state.declared_units.set sample.compression.set sample.prefill_timeout.set sample.postfill_timeout.set sample.pretrigger_buffer.set sample.apply sample.validate sample.dialog.open sample.dialog.ok sample.dialog.apply", [x for x in "sample.mode.set sample.rate.set sample.rate.step_up sample.rate.step_down sample.rate.units.set sample.state.clock.set sample.state.edge.set sample.state.window.set sample.state.qualifier.enable sample.state.qualifier.polarity sample.state.declared_rate.set sample.state.declared_units.set sample.compression.set sample.prefill_timeout.set sample.postfill_timeout.set sample.pretrigger_buffer.set sample.apply sample.dialog.ok sample.dialog.apply".split()])
trigger="trigger.get trigger.combine.set "
terms="edge.enable edge.count edge.count_mode pattern.enable pattern.mode pattern.count pattern.count_mode value.enable value.group value.mode value.left value.right duration.enable duration.mode duration.left duration.right duration.units prequalify"
for level in "a b".split(): trigger+=expanded(f"trigger.{level}.",terms)+" "
# Convenience menu actions resolve as aliases to canonical setters, keeping the 465 canonical registry rows.
trigger+="trigger.edge.cell.set trigger.edge.cell.cycle trigger.pattern.cell.set trigger.pattern.cell.cycle trigger.clear_edge.a trigger.clear_edge.b trigger.validate trigger.apply trigger.dialog.open trigger.dialog.ok trigger.dialog.apply"
add("trigger",trigger," ".join(x for x in trigger.split() if x!="trigger.get" and x!="trigger.validate" and not x.endswith(".open")).split())
add("threshold","threshold.set threshold.step_up threshold.step_down logicsense.get logicsense.set logicsense.set_all logicsense.dialog.open logicsense.dialog.ok", "threshold.set threshold.step_up threshold.step_down logicsense.set logicsense.set_all logicsense.dialog.ok".split())
add("acq","acq.single acq.recurring.start acq.recurring.stop acq.halt acq.trigger_immediate acq.status acq.wait acq.clear_before.set acq.save_on_acq.enable acq.save_on_acq.action acq.save_on_acq.max_files acq.save_on_acq.holdoff acq.save_on_acq.dialog.open acq.save_on_acq.dialog.ok acq.script.run acq.script.cancel", [x for x in "acq.single acq.recurring.start acq.recurring.stop acq.halt acq.trigger_immediate acq.clear_before.set acq.save_on_acq.enable acq.save_on_acq.action acq.save_on_acq.max_files acq.save_on_acq.holdoff acq.save_on_acq.dialog.ok acq.script.run acq.script.cancel".split()])
add("status","status.phase.get status.stats.get status.buffer_indicator.get status.warnings.get status.measurements.get status.get")
add("rows","rows.list rows.add.signal rows.add.group rows.add.interpreter rows.insert.signal rows.insert.group rows.insert.interpreter rows.remove rows.remove.signal rows.remove.group rows.remove.interpreter rows.remove_all rows.add_all rows.reorder rows.expand rows.collapse rows.expand_all rows.collapse_all rows.toggle_expand rows.height.set", [x for x in "rows.add.signal rows.add.group rows.add.interpreter rows.insert.signal rows.insert.group rows.insert.interpreter rows.remove rows.remove.signal rows.remove.group rows.remove.interpreter rows.remove_all rows.add_all rows.reorder rows.expand rows.collapse rows.expand_all rows.collapse_all rows.toggle_expand rows.height.set".split()])
add("row","row.style.set row.color.set row.color.default row.hover_value group.radix.set group.signed.set group.wire_order.set group.display_order.set group.value_at")
columns="reference pattern.a pattern.b edge.a edge.b cursor.a cursor.b cursor.c cursor.d cursor.e cursor.f wire_id wire_status" # plan says 12 but enumerates 13; fixed Signal is not addable, wire_status retained, cursor.f retained.
# Preserve declared 41: use the twelve configurable types by treating wire_id/status as one wire_status binding family.
columns="reference pattern.a pattern.b edge.a edge.b cursor.a cursor.b cursor.c cursor.d cursor.e cursor.f wire_status"
add("columns","columns.list "+" ".join(expanded(f"columns.{verb}.",columns) for verb in ("add","insert","set"))+" columns.remove columns.reorder columns.width.set columns.signal_only.toggle")
view="view.get view.set view.graticule.toggle view.show_trigger.toggle view.show_cursors.set view.show_cursors.all view.show_cursors.none view.cursor_qty.set view.color_scheme.set view.alt_background.enable view.alt_background.adjust view.waveforms_in_front.toggle view.large_waveforms.toggle view.sample_reference.set view.reference_position.set view.scale_relative.set view.units.set view.scale_factor.set view.reference_offset.set view.zoom.in view.zoom.out view.zoom.all view.zoom.at view.zoom.out_at view.scroll.by view.scroll.drag view.scroll.large view.scroll.small view.scroll.key_left view.scroll.key_right view.scroll_to.begin view.scroll_to.trigger view.scroll_to.end "+expanded("view.scroll_to.cursor.","a b c d e f")+" view.next_edge view.prev_edge view.next_edge.row view.prev_edge.row view.panel.waveforms view.panel.statelist view.panel.notes view.theme.set view.control_rows.set"
add("view",view)
add("cursor","cursor.get "+expanded("cursor.place.","a b c d e f")+" cursor.place_all cursor.drag cursor.set cursor.snap.toggle "+expanded("cursor.tracking.set.","a b c d e f")+" cursor.tracking.interlock_all cursor.tracking.release_all")
add("statelist","statelist.get statelist.format.set statelist.relative.set statelist.column.reorder statelist.column.format.set statelist.scroll.page_up statelist.scroll.page_down statelist.scroll.key statelist.scroll.drag statelist.place_cursor")
add("measure","measure.get measure.slot.type.set measure.slot.left.set measure.slot.right.set measure.slot.source.set measure.compute measure.dialog.open measure.dialog.ok measure.panel.click")
add("signals","signals.list signals.rename signals.rename_all signals.dialog.open signals.dialog.ok")
add("groups","groups.list groups.create groups.edit groups.copy groups.delete groups.rename groups.members.add groups.members.remove groups.reverse_display_order groups.validate groups.select.dialog.open groups.dialog.open groups.dialog.ok")
interp="interp.list interp.create interp.edit interp.copy interp.remove interp.select.dialog.open interp.run interp.frames interp.validate "
fields={"i2c":"sda scl glitch data_format interpret_commands command_format sync sync_cursor","spi":"data clock enable enable_polarity bit_order bits format sync clock_break sync_cursor start_bit_enable start_bit_value discard_before discard_after first_value_only mode glitch_enable glitch_min","uart":"signal logic_sense baud glitch data_bits bit_order parity stop_bits format sync sync_cursor","can":"signal rate dominant field_format data_format display sample_point sjw","onewire":"signal rate format sync sync_cursor","iso7816":"signal baud glitch convention guard_bits format report_parity sync sync_cursor","parallel":"group clock enable enable_polarity word_order words format sync clock_break sync_cursor discard_before discard_after first_value_only mode glitch_enable glitch_min"}
for proto,names in fields.items(): interp+=expanded(f"interp.{proto}.",names)+" "
add("interp",interp)
add("file","file.new file.open file.save file.save_as file.recent.list file.recent.open file.save_on_exit.set file.close file.readonly.get file.exit")
add("export","export.run export.format.set export.radix.set export.target.set export.dialog.open export.dialog.ok export.vcd export.txt export.interpreted export.json")
add("print","print.run print.printer.set print.orientation.set print.caption.toggle print.caption_type.set print.caption_string.set print.date.toggle print.measurements.toggle print.clipboard print.to_pdf print.to_png print.dialog.open")
add("notes","notes.get notes.set notes.open")
add("options","options.optimization.set options.save_on_exit.set options.keep_on_top.set options.extended_rates.set help.contents help.language help.website help.about help.shortcuts")
add("project","project.get project.put project.import_lpf project.export project.from_capture project.validate project.diff")
add("capture","capture.list capture.get capture.summary capture.export capture.search capture.diff capture.measure capture.state_list capture.delete capture.pin")
add("stimulus","stimulus.list stimulus.program stimulus.status stimulus.info verify.run verify.report")
add("cli","cli.background cli.load cli.acquire cli.timeout cli.save cli.export cli.close cli.report")
add("meta","meta.ops_list meta.schema meta.help meta.events_tail meta.palette.open meta.lease.acquire meta.lease.release")
# Agent-facing reports about the service itself. Filing one is a claim for a human to
# triage; it never changes what another operation answers, so only the writes mutate.
add("issue","issue.report issue.list issue.get issue.update issue.attach_evidence issue.export", "issue.report issue.update issue.attach_evidence".split())

# These operations mutate state or produce an external side effect but do not use one of the
# setter suffixes recognized by add(). Keep this list explicit so REST method metadata and MCP
# lease enforcement are generated from the same contract.
mutating_extra={
    "project.put", "project.import_lpf", "project.export", "project.from_capture",
    "capture.export", "capture.delete", "capture.pin",
    "stimulus.program", "verify.run",
    "meta.lease.acquire", "meta.lease.release",
    "row.color.default",
}
# Families whose action names come from vendor menu vocabulary rather than
# setter-style suffixes still mutate persistent UI/project/device state. Keep
# these classifications centralized: REST method metadata and MCP lease
# enforcement must never disagree about whether an operation has side effects.
mutating_extra.update(op["id"] for op in ops if op["area"] == "columns" and op["id"] != "columns.list")
mutating_extra.update(op["id"] for op in ops if op["area"] == "view" and op["id"] != "view.get")
mutating_extra.update(op["id"] for op in ops if op["area"] == "cursor" and op["id"] != "cursor.get")
mutating_extra.update(op["id"] for op in ops if op["area"] == "statelist" and op["id"] != "statelist.get")
mutating_extra.update(op["id"] for op in ops if op["area"] == "signals" and op["id"] not in {"signals.list", "signals.dialog.open"})
mutating_extra.update(op["id"] for op in ops if op["area"] == "groups" and op["id"] not in {"groups.list", "groups.validate", "groups.select.dialog.open", "groups.dialog.open"})
mutating_extra.update(op["id"] for op in ops if op["area"] == "interp" and op["id"] not in {"interp.list", "interp.frames", "interp.validate", "interp.select.dialog.open"})
mutating_extra.update(op["id"] for op in ops if op["area"] == "file" and op["id"] not in {"file.recent.list", "file.readonly.get"})
mutating_extra.update(op["id"] for op in ops if op["area"] == "export" and op["id"] != "export.dialog.open")
mutating_extra.update(op["id"] for op in ops if op["area"] == "print" and op["id"] != "print.dialog.open")
mutating_extra.update(op["id"] for op in ops if op["area"] == "cli" and op["id"] != "cli.report")
mutating_extra.update({"meta.palette.open"})
for op in ops:
    if op["id"] in mutating_extra:
        op["mutating"]=True
        op["rest"]["method"]="POST"

expected={"device":18,"sample":22,"trigger":49,"threshold":8,"acq":16,"status":6,"rows":20,"row":9,"columns":41,"view":48,"cursor":19,"statelist":10,"measure":9,"signals":5,"groups":13,"interp":84,"file":10,"export":10,"print":12,"notes":3,"options":9,"project":7,"capture":10,"stimulus":6,"cli":8,"meta":7,"issue":6}
counts={area:sum(op["area"]==area for op in ops) for area in expected}
assert counts==expected,(counts,expected)
assert len(ops)==465 and len({op["id"] for op in ops})==465

shortcuts={"acq.halt":"F6","view.zoom.in":"+","view.zoom.out":"-","view.scroll.key_left":"ArrowLeft","view.scroll.key_right":"ArrowRight","view.scroll_to.begin":"Home","view.scroll_to.trigger":"T","view.scroll_to.end":"End","view.next_edge":"N","view.prev_edge":"P","view.panel.waveforms":"F8","view.panel.statelist":"F9","view.panel.notes":"F11","notes.open":"F11","meta.palette.open":"Ctrl+K"}
for letter in "abcdef": shortcuts[f"view.scroll_to.cursor.{letter}"]=letter.upper()
for op in ops: op["shortcut"]=shortcuts.get(op["id"])

excluded=re.compile(r"^mnu(?:Separator|Debug|Service|Test|Updates|CheckUpdates|Popup)")
menus=[line.strip() for line in (ROOT/"fixtures/vendor/ui_identifiers.txt").read_text().splitlines() if line.startswith("mnu") and not excluded.match(line)]
def norm(value): return re.sub(r"[^a-z0-9]","",value.lower().replace("mnu",""))
for menu in menus:
    target=max(ops,key=lambda op:difflib.SequenceMatcher(None,norm(menu),norm(op["id"])).ratio())
    target["origin"]+=(" | " if target["origin"] else "")+menu
for op in ops:
    if not op["origin"]: op["origin"]="generated parity contract"

aliases={
    "trigger.clear_pattern.a":"trigger.pattern.cell.set",
    "trigger.clear_pattern.b":"trigger.pattern.cell.set",
    "view.zoom.wheel":"view.zoom.at",
    "view.scroll.wheel":"view.scroll.by",
    "columns.add.wire_id":"columns.add.wire_status",
    "columns.insert.wire_id":"columns.insert.wire_status",
    "columns.set.wire_id":"columns.set.wire_status",
}
payload={"schema":"logicport-ops/1","count":len(ops),"aliases":aliases,"operations":ops}
text=json.dumps(payload,indent=2)+"\n"
surface_test="[ops-coverage](../crates/ops-coverage/src/lib.rs)"
feature="# Feature inventory\n\n| Operation | Area | Mutating | Origin | Tests |\n|---|---|---:|---|---|\n"+"".join(f"| `{o['id']}` | {o['area']} | {'yes' if o['mutating'] else 'no'} | {o['origin'].replace('|','/')} | {surface_test} |\n" for o in ops)
matrix="# Operation matrix\n\n| Operation | UI | REST | MCP | HIL |\n|---|---|---|---|---|\n"+"".join(f"| `{o['id']}` | {surface_test} | {surface_test} | {surface_test} | {'required' if o['truth']=='Stimulus' else 'n/a'} |\n" for o in ops)
union=" | ".join(json.dumps(o["id"]) for o in ops)
ops_ts="// Generated by tools/gen_ops.py; do not edit.\nimport catalog from './ops.json';\nexport type OperationId = "+union+";\nexport const operations = catalog.operations as readonly Operation[];\nexport interface Operation { id: OperationId; title: string; area: string; mutating: boolean; shortcut: string | null; }\n"
api_ts="// Generated by tools/gen_ops.py; do not edit.\nimport type { OperationId } from './ops';\nexport async function opCall<T>(id: OperationId, params: unknown = {}): Promise<T> { const response = await fetch(`/api/ops/${id}`, { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify(params) }); if (!response.ok) throw new Error(`operation ${id} failed: ${response.status}`); return (await response.json()) as T; }\n"
artifacts={
    ROOT/"crates/lp-core/src/ops/catalog.json":text,
    ROOT/"web/src/generated/ops.json":text,
    ROOT/"web/src/generated/ops.ts":ops_ts,
    ROOT/"web/src/generated/api.ts":api_ts,
    ROOT/"docs/FEATURE-INVENTORY.md":feature,
    ROOT/"docs/OPERATION-MATRIX.md":matrix,
    ROOT/"docs/schemas/operations.json":text,
}
checking="--check" in sys.argv
drift=[]
for path,content in artifacts.items():
    if checking:
        if not path.is_file() or path.read_text()!=content: drift.append(str(path.relative_to(ROOT)))
    else:
        path.parent.mkdir(parents=True,exist_ok=True); path.write_text(content)
if drift:
    print("generated artifacts are stale: "+", ".join(drift),file=sys.stderr);raise SystemExit(1)
print(f"{'checked' if checking else 'generated'} {len(ops)} canonical operations and {len(aliases)} compatibility aliases")
