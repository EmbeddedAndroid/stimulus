import { useEffect, useMemo, useState } from "react";
import { operations, type Operation, type OperationId } from "./generated/ops";

export function matchingOperations(query: string): readonly Operation[] {
  const needle = query.trim().toLowerCase();
  if (needle === "") return operations;
  return operations.filter((operation) =>
    `${operation.id} ${operation.title} ${operation.area}`.toLowerCase().includes(needle),
  );
}

interface Props {
  open: boolean;
  busy: boolean;
  onClose: () => void;
  onInvoke: (id: OperationId, params: unknown) => Promise<void>;
}

export function CommandPalette({ open, busy, onClose, onInvoke }: Props) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Operation | null>(null);
  const [params, setParams] = useState("{}");
  const [paramsError, setParamsError] = useState<string | null>(null);
  const matches = useMemo(() => matchingOperations(query), [query]);

  useEffect(() => {
    if (!open) {
      setQuery("");
      setSelected(null);
      setParams("{}");
      setParamsError(null);
    }
  }, [open]);

  if (!open) return null;
  const invoke = async () => {
    if (selected === null) return;
    try {
      const parsed: unknown = JSON.parse(params);
      if (parsed === null || Array.isArray(parsed) || typeof parsed !== "object") {
        throw new Error("Parameters must be a JSON object");
      }
      setParamsError(null);
      await onInvoke(selected.id, parsed);
      onClose();
    } catch (cause: unknown) {
      setParamsError(cause instanceof Error ? cause.message : String(cause));
    }
  };

  return <div className="modal-backdrop palette-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
    <section className="command-palette" role="dialog" aria-modal="true" aria-label="Operations">
      <header><input autoFocus aria-label="Search operations" placeholder="Search all 459 operations…" value={query} onChange={(event) => { setQuery(event.target.value); setSelected(null); }} /><kbd>Esc</kbd></header>
      <div className="palette-body">
        <div className="palette-results" role="listbox" aria-label="Operation results">
          {matches.map((operation) => <button type="button" role="option" aria-selected={selected?.id === operation.id} key={operation.id} onClick={() => setSelected(operation)}>
            <span><strong>{operation.title}</strong><small>{operation.id}</small></span><em>{operation.area}</em>
          </button>)}
        </div>
        <form className="palette-detail" onSubmit={(event) => { event.preventDefault(); void invoke(); }}>
          {selected === null ? <p>Select an operation to inspect and run it.</p> : <>
            <h2>{selected.title}</h2><code>{selected.id}</code>
            <label>Parameters (JSON)<textarea value={params} onChange={(event) => setParams(event.target.value)} spellCheck={false} /></label>
            {paramsError !== null && <p className="field-error" role="alert">{paramsError}</p>}
            <button className="primary" type="submit" disabled={busy}>Run operation</button>
          </>}
        </form>
      </div>
    </section>
  </div>;
}
