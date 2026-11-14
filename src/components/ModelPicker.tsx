import { useEffect, useId, useMemo, useRef, useState } from "react";
import { ChevronDown, Search, X } from "lucide-react";
import type { Provider } from "../types";
import { filterModels, modelOptions } from "./modelSearch";
import type { ModelOption } from "./modelSearch";
import "./modelPicker.css";

interface ModelPickerProps {
  providers: Provider[];
  providerId: string;
  model: string;
  onChange: (selection: { providerId: string; model: string }) => void;
  disabled?: boolean;
  required?: boolean;
  label?: string;
  description?: string;
}

export function ModelPicker({
  providers,
  providerId,
  model,
  onChange,
  disabled = false,
  required = false,
  label = "Model",
  description = "Search by model or provider. Use * and ? as wildcards.",
}: ModelPickerProps) {
  const id = useId();
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const editing = useRef(false);
  const initialSelection = useRef({ providerId, model });
  const options = useMemo(
    () => modelOptions(providers, model ? { providerId, model } : initialSelection.current),
    [providers, providerId, model],
  );
  const selected = options.find((option) => option.providerId === providerId && option.model === model);
  const [query, setQuery] = useState(selected?.label ?? "");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const selectedLabel = selected?.label ?? "";
  const matches = useMemo(() => filterModels(options, editing.current ? query : ""), [options, query, open]);

  useEffect(() => {
    if (selectedLabel || !editing.current) {
      setQuery(selectedLabel);
      editing.current = false;
    }
  }, [selectedLabel, providerId, model]);

  useEffect(() => {
    input.current?.setCustomValidity(!selected && (required || query.trim().length > 0) ? "Choose a model from the search results." : "");
  }, [required, selected, query]);

  useEffect(() => {
    if (open) list.current?.children[active]?.scrollIntoView({ block: "nearest" });
  }, [active, open]);

  function choose(option: ModelOption) {
    editing.current = false;
    setQuery(option.label);
    setOpen(false);
    setActive(0);
    onChange({ providerId: option.providerId, model: option.model });
  }

  function clear() {
    editing.current = true;
    setQuery("");
    setActive(0);
    setOpen(true);
    onChange({ providerId: "", model: "" });
    input.current?.focus();
  }

  return (
    <div className="model-picker field">
      <label htmlFor={`${id}-input`}>{label}</label>
      <div className={`model-picker-control ${open ? "is-open" : ""}`}>
        <Search size={16} aria-hidden="true" />
        <input
          id={`${id}-input`}
          ref={input}
          role="combobox"
          aria-autocomplete="list"
          aria-expanded={open}
          aria-controls={`${id}-results`}
          aria-activedescendant={open && matches[active] ? `${id}-option-${active}` : undefined}
          aria-describedby={`${id}-hint`}
          autoComplete="off"
          spellCheck={false}
          required={required}
          disabled={disabled}
          value={query}
          placeholder="Search all models…"
          onFocus={() => {
            setOpen(true);
            const index = matches.findIndex((option) => option.providerId === providerId && option.model === model);
            setActive(Math.max(0, index));
          }}
          onBlur={() => setOpen(false)}
          onChange={(event) => {
            editing.current = true;
            setQuery(event.target.value);
            setActive(0);
            setOpen(true);
            if (providerId || model) onChange({ providerId: "", model: "" });
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape" && open) {
              event.preventDefault();
              event.stopPropagation();
              setOpen(false);
            } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              setOpen(true);
              const direction = event.key === "ArrowDown" ? 1 : -1;
              setActive((current) => !open ? (direction > 0 ? 0 : Math.max(0, matches.length - 1)) : Math.max(0, Math.min(matches.length - 1, current + direction)));
            } else if (event.key === "Enter" && open) {
              event.preventDefault();
              if (matches[active]) choose(matches[active]);
            }
          }}
        />
        {query && !disabled ? (
          <button type="button" className="model-picker-clear" aria-label="Clear model selection" onMouseDown={(event) => event.preventDefault()} onClick={clear}><X size={14} /></button>
        ) : <ChevronDown size={15} aria-hidden="true" />}
      </div>
      {open && !disabled && (
        <div className="model-picker-popup">
          <div className="model-picker-result-count" role="status">{matches.length} {matches.length === 1 ? "model" : "models"}{editing.current && query.trim() ? " match" : " available"}</div>
          <div id={`${id}-results`} role="listbox" aria-label="Available models" className="model-picker-options" ref={list}>
            {matches.map((option, index) => (
              <button
                type="button"
                role="option"
                id={`${id}-option-${index}`}
                key={`${option.providerId}:${option.model}`}
                tabIndex={-1}
                aria-selected={index === active}
                className={index === active ? "is-active" : ""}
                onMouseDown={(event) => event.preventDefault()}
                onPointerMove={() => setActive(index)}
                onClick={() => choose(option)}
              >{option.label}</button>
            ))}
          </div>
          {!matches.length && <p className="model-picker-empty">{options.length ? "No matching models. Try a shorter search or *." : "Add a connection and discover models in Connections."}</p>}
        </div>
      )}
      <small id={`${id}-hint`} className="model-picker-hint">{description}</small>
    </div>
  );
}
