# Feature inventory

| Operation | Area | Mutating | Origin | Tests |
|---|---|---:|---|---|
| `device.enumerate` | device | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.connect` | device | yes | mnuDtoNone / mnuEtoNone | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.disconnect` | device | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.demo.enter` | device | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.demo.exit` | device | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.status` | device | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.diagnose` | device | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.selftest` | device | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.usb_error_count.get` | device | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.usb_error_count.reset` | device | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.regs.read` | device | no | mnuOverhead | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.regs.write` | device | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.pins.read` | device | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.fpga.configure` | device | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.transcript.start` | device | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.transcript.stop` | device | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.wire_status.read` | device | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `device.freq_counter.read` | device | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.get` | sample | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.mode.set` | sample | yes | mnuSetupSampleMode / mnuSetupSampleMode_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.rate.set` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.rate.step_up` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.rate.step_down` | sample | yes | mnuSetAlternateColor | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.rate.units.set` | sample | yes | mnuScaleUnitsSamples / mnuScaleUnitsTime | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.state.clock.set` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.state.edge.set` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.state.window.set` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.state.qualifier.enable` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.state.qualifier.polarity` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.state.declared_rate.set` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.state.declared_units.set` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.compression.set` | sample | yes | mnuDisplayOptions | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.prefill_timeout.set` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.postfill_timeout.set` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.pretrigger_buffer.set` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.apply` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.validate` | sample | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.dialog.open` | sample | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.dialog.ok` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `sample.dialog.apply` | sample | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.get` | trigger | no | mnuSetupTrigger | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.combine.set` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.edge.enable` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.edge.count` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.edge.count_mode` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.pattern.enable` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.pattern.mode` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.pattern.count` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.pattern.count_mode` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.value.enable` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.value.group` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.value.mode` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.value.left` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.value.right` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.duration.enable` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.duration.mode` | trigger | yes | mnuAtoNone | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.duration.left` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.duration.right` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.duration.units` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.a.prequalify` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.edge.enable` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.edge.count` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.edge.count_mode` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.pattern.enable` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.pattern.mode` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.pattern.count` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.pattern.count_mode` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.value.enable` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.value.group` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.value.mode` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.value.left` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.value.right` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.duration.enable` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.duration.mode` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.duration.left` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.duration.right` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.duration.units` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.b.prequalify` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.edge.cell.set` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.edge.cell.cycle` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.pattern.cell.set` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.pattern.cell.cycle` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.clear_edge.a` | trigger | yes | mnuClearEdge | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.clear_edge.b` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.validate` | trigger | no | mnuSetupTrigger_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.apply` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.dialog.open` | trigger | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.dialog.ok` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `trigger.dialog.apply` | trigger | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `threshold.set` | threshold | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `threshold.step_up` | threshold | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `threshold.step_down` | threshold | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `logicsense.get` | threshold | no | mnuLogicSense | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `logicsense.set` | threshold | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `logicsense.set_all` | threshold | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `logicsense.dialog.open` | threshold | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `logicsense.dialog.ok` | threshold | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.single` | acq | yes | mnuAcquisition | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.recurring.start` | acq | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.recurring.stop` | acq | yes | mnuHaltAcquisition / mnuRecurringAcquisition | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.halt` | acq | yes | mnuAto | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.trigger_immediate` | acq | yes | mnuTriggerImmediate | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.status` | acq | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.wait` | acq | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.clear_before.set` | acq | yes | mnuClearBeforeAcq | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.save_on_acq.enable` | acq | yes | mnuSaveOnAcqEnable | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.save_on_acq.action` | acq | yes | mnuSaveOnAcquisition | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.save_on_acq.max_files` | acq | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.save_on_acq.holdoff` | acq | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.save_on_acq.dialog.open` | acq | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.save_on_acq.dialog.ok` | acq | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.script.run` | acq | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `acq.script.cancel` | acq | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `status.phase.get` | status | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `status.stats.get` | status | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `status.buffer_indicator.get` | status | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `status.warnings.get` | status | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `status.measurements.get` | status | no | mnuSetupMeasurements / mnuSetupMeasurements_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `status.get` | status | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.list` | rows | no | mnuLSB_Top / mnuRows | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.add.signal` | rows | yes | mnuShowSignals / mnuSignalAddSignal | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.add.group` | rows | yes | mnuExpandGroups / mnuSignalAddGroup | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.add.interpreter` | rows | yes | mnuSignalAddInterpreter | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.insert.signal` | rows | yes | mnuEditSignal / mnuInsertSignal / mnuSetupSignals | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.insert.group` | rows | yes | mnuEditGroup / mnuInsertGroup / mnuSetup / mnuSetupGroups | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.insert.interpreter` | rows | yes | mnuInsertInterpreter / mnuSetupInterpreters | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.remove` | rows | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.remove.signal` | rows | yes | mnuRemoveSignal | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.remove.group` | rows | yes | mnuRemoveColumn / mnuRemoveGroup | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.remove.interpreter` | rows | yes | mnuRemoveInterpreter | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.remove_all` | rows | yes | mnuRemoveAll | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.add_all` | rows | yes | mnuAddAll | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.reorder` | rows | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.expand` | rows | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.collapse` | rows | yes | mnuCollapseGroups | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.expand_all` | rows | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.collapse_all` | rows | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.toggle_expand` | rows | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `rows.height.set` | rows | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `row.style.set` | row | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `row.color.set` | row | yes | mnuWaveColor | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `row.color.default` | row | yes | mnuWaveDefaultColor | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `row.hover_value` | row | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `group.radix.set` | row | yes | mnuRadix | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `group.signed.set` | row | yes | mnuSignedData | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `group.wire_order.set` | row | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `group.display_order.set` | row | yes | mnuDisplayOrder | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `group.value_at` | row | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.list` | columns | no | mnuColumns / mnuMSB_LSB / mnuSetColumn | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.reference` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.pattern.a` | columns | yes | mnuClearPattern | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.pattern.b` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.edge.a` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.edge.b` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.cursor.a` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.cursor.b` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.cursor.c` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.cursor.d` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.cursor.e` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.cursor.f` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.add.wire_status` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.reference` | columns | yes | mnuColumnsInsertReference / mnuSignalInsertReference | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.pattern.a` | columns | yes | mnuColumnsInsertAPattern / mnuColumnsInsertPattern / mnuSignalInsertAPattern / mnuSignalInsertPattern | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.pattern.b` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.edge.a` | columns | yes | mnuColumnsInsertAnEdge / mnuColumnsInsertEdge / mnuColumnsInsertWireID | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.edge.b` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.cursor.a` | columns | yes | mnuColumnsInsertACursor / mnuColumnsInsertColumn / mnuColumnsInsertCursor / mnuSignalInsertACursor / mnuSignalInsertCursor | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.cursor.b` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.cursor.c` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.cursor.d` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.cursor.e` | columns | yes | mnuPositionAllCursorsHere / mnuPositionCursorHere | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.cursor.f` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.insert.wire_status` | columns | yes | mnuColumnsInsertWireStatus / mnuSignalInsertWireStatus | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.reference` | columns | yes | mnuColumnsSetReference / mnuSelectDisplayReference | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.pattern.a` | columns | yes | mnuColumnsSetPattern / mnuColumnsSetThePattern | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.pattern.b` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.edge.a` | columns | yes | mnuColumnsSetEdge / mnuColumnsSetTheEdge | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.edge.b` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.cursor.a` | columns | yes | mnuColumnsSetCursor / mnuColumnsSetTheCursor | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.cursor.b` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.cursor.c` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.cursor.d` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.cursor.e` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.cursor.f` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.set.wire_status` | columns | yes | mnuColumnsSetWireID / mnuColumnsSetWireStatus | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.remove` | columns | yes | mnuCtoNone | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.reorder` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.width.set` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `columns.signal_only.toggle` | columns | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.get` | view | no | mnuView | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.set` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.graticule.toggle` | view | yes | mnuGraticule | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.show_trigger.toggle` | view | yes | mnuShowTrigger | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.show_cursors.set` | view | yes | mnuShowAllCursors / mnuShowCursors / mnuShowNoCursors | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.show_cursors.all` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.show_cursors.none` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.cursor_qty.set` | view | yes | mnuSetCursorQty | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.color_scheme.set` | view | yes | mnuColorScheme / mnuSelectColorScheme | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.alt_background.enable` | view | yes | mnuEnableAlternateBackground | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.alt_background.adjust` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.waveforms_in_front.toggle` | view | yes | mnuWaveformsInFront | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.large_waveforms.toggle` | view | yes | mnuChooseWaveformStyle / mnuLargeWaveforms / mnuWaveformColor / mnuWaveformStyle | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.sample_reference.set` | view | yes | mnuDisplayReference / mnuSampleReference / mnuScaleReference / mnuSelectSampleReference | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.reference_position.set` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scale_relative.set` | view | yes | mnuScaleRelative | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.units.set` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scale_factor.set` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.reference_offset.set` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.zoom.in` | view | yes | mnuZoomIn / mnuZoomIn_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.zoom.out` | view | yes | mnuZoomOut / mnuZoomOut_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.zoom.all` | view | yes | mnuZoomAll / mnuZoomAll_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.zoom.at` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.zoom.out_at` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll.by` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll.drag` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll.large` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll.small` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll.key_left` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll.key_right` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll_to.begin` | view | yes | mnuScrollToBegin / mnuScrollToBegin_Click / mnuViewScrollToBegin | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll_to.trigger` | view | yes | mnuScaleTrigger / mnuScrollToTrigger / mnuScrollToTrigger_Click / mnuViewScrollToTrigger | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll_to.end` | view | yes | mnuScrollToEnd / mnuScrollToEnd_Click / mnuViewScrollToEnd | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll_to.cursor.a` | view | yes | mnuiScrollCursor / mnuScrollToCursor / mnuViewScrollCursor / mnuViewScrollToCursor | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll_to.cursor.b` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll_to.cursor.c` | view | yes | mnuScrollToCursor_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll_to.cursor.d` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll_to.cursor.e` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.scroll_to.cursor.f` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.next_edge` | view | yes | mnuNextEdge / mnuNextEdge_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.prev_edge` | view | yes | mnuPreviousEdge / mnuPreviousEdge_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.next_edge.row` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.prev_edge.row` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.panel.waveforms` | view | yes | mnuWaveform / mnuWaveforms | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.panel.statelist` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.panel.notes` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.theme.set` | view | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `view.control_rows.set` | view | yes | mnuControlRows / mnuSelectControlRows | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.get` | cursor | no | mnuCursorQty | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.place.a` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.place.b` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.place.c` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.place.d` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.place.e` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.place.f` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.place_all` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.drag` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.set` | cursor | yes | mnuToggleCursors | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.snap.toggle` | cursor | yes | mnuCursorSnap | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.tracking.set.a` | cursor | yes | mnuCursorTracking / mnuCursorTracks | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.tracking.set.b` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.tracking.set.c` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.tracking.set.d` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.tracking.set.e` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.tracking.set.f` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.tracking.interlock_all` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cursor.tracking.release_all` | cursor | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `statelist.get` | statelist | no | mnuStateList / mnuStateListData | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `statelist.format.set` | statelist | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `statelist.relative.set` | statelist | yes | mnuStateListStates | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `statelist.column.reorder` | statelist | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `statelist.column.format.set` | statelist | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `statelist.scroll.page_up` | statelist | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `statelist.scroll.page_down` | statelist | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `statelist.scroll.key` | statelist | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `statelist.scroll.drag` | statelist | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `statelist.place_cursor` | statelist | yes | mnuPlaceCursor / mnuStateListSamples | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `measure.get` | measure | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `measure.slot.type.set` | measure | yes | mnuMSB_Bottom | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `measure.slot.left.set` | measure | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `measure.slot.right.set` | measure | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `measure.slot.source.set` | measure | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `measure.compute` | measure | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `measure.dialog.open` | measure | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `measure.dialog.ok` | measure | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `measure.panel.click` | measure | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `signals.list` | signals | no | mnuEditSignals / mnuShowSignals_Click / mnuSignal / mnuSignalInsertAnEdge / mnuSignalInsertEdge / mnuSignalInsertWireID / mnuSingleAcquisition | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `signals.rename` | signals | yes | mnuSignalAddColumn | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `signals.rename_all` | signals | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `signals.dialog.open` | signals | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `signals.dialog.ok` | signals | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.list` | groups | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.create` | groups | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.edit` | groups | yes | mnuEdit | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.copy` | groups | yes | mnuCopy | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.delete` | groups | yes | mnuDelete / mnuRemoveSelected | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.rename` | groups | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.members.add` | groups | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.members.remove` | groups | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.reverse_display_order` | groups | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.validate` | groups | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.select.dialog.open` | groups | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.dialog.open` | groups | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `groups.dialog.ok` | groups | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.list` | interp | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.create` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.edit` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.copy` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.remove` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.select.dialog.open` | interp | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.run` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.frames` | interp | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.validate` | interp | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.i2c.sda` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.i2c.scl` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.i2c.glitch` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.i2c.data_format` | interp | yes | mnuDataFormat | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.i2c.interpret_commands` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.i2c.command_format` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.i2c.sync` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.i2c.sync_cursor` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.data` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.clock` | interp | yes | mnuNotes_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.enable` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.enable_polarity` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.bit_order` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.bits` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.format` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.sync` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.clock_break` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.sync_cursor` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.start_bit_enable` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.start_bit_value` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.discard_before` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.discard_after` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.first_value_only` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.mode` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.glitch_enable` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.spi.glitch_min` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.signal` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.logic_sense` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.baud` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.glitch` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.data_bits` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.bit_order` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.parity` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.stop_bits` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.format` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.sync` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.uart.sync_cursor` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.can.signal` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.can.rate` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.can.dominant` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.can.field_format` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.can.data_format` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.can.display` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.can.sample_point` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.can.sjw` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.onewire.signal` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.onewire.rate` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.onewire.format` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.onewire.sync` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.onewire.sync_cursor` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.iso7816.signal` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.iso7816.baud` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.iso7816.glitch` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.iso7816.convention` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.iso7816.guard_bits` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.iso7816.format` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.iso7816.report_parity` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.iso7816.sync` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.iso7816.sync_cursor` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.group` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.clock` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.enable` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.enable_polarity` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.word_order` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.words` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.format` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.sync` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.clock_break` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.sync_cursor` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.discard_before` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.discard_after` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.first_value_only` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.mode` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.glitch_enable` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `interp.parallel.glitch_min` | interp | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `file.new` | file | yes | mnuFile / mnuFileNew / mnuFileNew_Click / mnuNew | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `file.open` | file | yes | mnuFileOpen / mnuFileOpen_Click / mnuFto | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `file.save` | file | yes | mnuFileSave / mnuFileSave_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `file.save_as` | file | yes | mnuFileSaveAs / mnuLSB_MSB | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `file.recent.list` | file | no | mnuRecentFile / mnuRecentFiles | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `file.recent.open` | file | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `file.save_on_exit.set` | file | yes | mnuSaveOnExit / mnuSaveOnExitOption | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `file.close` | file | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `file.readonly.get` | file | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `file.exit` | file | yes | mnuExit / mnuFileExport | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `export.run` | export | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `export.format.set` | export | yes | mnuPerformance | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `export.radix.set` | export | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `export.target.set` | export | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `export.dialog.open` | export | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `export.dialog.ok` | export | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `export.vcd` | export | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `export.txt` | export | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `export.interpreted` | export | yes | mnuEditInterpreter | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `export.json` | export | yes | mnuEto | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.run` | print | yes | mnuPrint | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.printer.set` | print | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.orientation.set` | print | yes | mnuBtoNone / mnuFtoNone | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.caption.toggle` | print | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.caption_type.set` | print | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.caption_string.set` | print | yes | mnuDataSettings | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.date.toggle` | print | yes | mnuDto | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.measurements.toggle` | print | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.clipboard` | print | yes | mnuPrint_Click / mnuPrintToClipbaord | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.to_pdf` | print | yes | mnuAppOnTop | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.to_png` | print | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `print.dialog.open` | print | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `notes.get` | notes | no | mnuNoneTrack / mnuNotes | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `notes.set` | notes | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `notes.open` | notes | no | mnuBto | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `options.optimization.set` | options | yes | mnuOptimize | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `options.save_on_exit.set` | options | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `options.keep_on_top.set` | options | yes | mnuOptions | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `options.extended_rates.set` | options | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `help.contents` | options | no | mnuHelpContents / mnuHelpContents_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `help.language` | options | no | mnuHelpLanguage / mnuHelpLanguageSelect | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `help.website` | options | no | mnuWebsite / mnuWebsite_Click | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `help.about` | options | no | mnuAbout | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `help.shortcuts` | options | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `project.get` | project | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `project.put` | project | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `project.import_lpf` | project | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `project.export` | project | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `project.from_capture` | project | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `project.validate` | project | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `project.diff` | project | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `capture.list` | capture | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `capture.get` | capture | no | mnuSaveOnCaptureSettings | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `capture.summary` | capture | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `capture.export` | capture | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `capture.search` | capture | no | mnuAllTrack | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `capture.diff` | capture | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `capture.measure` | capture | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `capture.state_list` | capture | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `capture.delete` | capture | yes | mnuAddSelected | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `capture.pin` | capture | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `stimulus.list` | stimulus | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `stimulus.program` | stimulus | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `stimulus.status` | stimulus | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `stimulus.info` | stimulus | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `verify.run` | stimulus | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `verify.report` | stimulus | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cli.background` | cli | yes | mnuAlternateBackground | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cli.load` | cli | yes | mnuGlobal | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cli.acquire` | cli | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cli.timeout` | cli | yes | mnuCto | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cli.save` | cli | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cli.export` | cli | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cli.close` | cli | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `cli.report` | cli | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `meta.ops_list` | meta | no | mnuMSB_Top | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `meta.schema` | meta | no | mnuMSB_MSB | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `meta.help` | meta | no | mnuHelp | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `meta.events_tail` | meta | no | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `meta.palette.open` | meta | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `meta.lease.acquire` | meta | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
| `meta.lease.release` | meta | yes | generated parity contract | [ops-coverage](../crates/ops-coverage/src/lib.rs) |
