import type { ParameterDef, ParamValue } from "@/domain/types";

export const fieldClass = "h-8 w-full rounded-md border border-border bg-card px-2 text-sm text-foreground";

const HINT: Record<string, string> = {
  host: "A host name, IP address or small CIDR range. Checked against your lab list before running.",
  ip: "An IP address. Checked against your lab list before running.",
  url: "An http:// or https:// address. Checked against your lab list before running.",
  port: "A port number from 1 to 65535.",
  port_list: "Ports separated by commas, or ranges such as 8000-8100. No spaces.",
  path: "A folder or file on this computer. System and credential folders are refused.",
};

/** One typed form field. The backend validates again; these hints only help people get it right the first time. */
export function ParamField({ def, value, onChange }: { def: ParameterDef; value: ParamValue | undefined; onChange: (v: ParamValue) => void }) {
  const id = `param-${def.name}`;
  const k = def.kind;
  const label = (
    <span className="mb-1 block text-xs text-muted-foreground">
      {def.label}
      {def.required ? " (required)" : ""}
    </span>
  );
  if (k.type === "boolean") {
    return (
      <label className="flex items-center gap-2 text-sm">
        <input type="checkbox" checked={value === true} onChange={(e) => onChange(e.target.checked)} />
        {def.label}
      </label>
    );
  }
  if (k.type === "choice") {
    return (
      <label htmlFor={id}>
        {label}
        <select id={id} className={fieldClass} value={String(value ?? "")} onChange={(e) => onChange(e.target.value)}>
          {k.options.map((o) => (
            <option key={o}>{o}</option>
          ))}
        </select>
        {def.description ? <span className="mt-0.5 block text-xs text-muted-foreground">{def.description}</span> : null}
      </label>
    );
  }
  if (k.type === "integer") {
    return (
      <label htmlFor={id}>
        {label}
        <input id={id} type="number" className={fieldClass} min={k.min} max={k.max} value={String(value ?? "")} onChange={(e) => onChange(Number(e.target.value))} />
        <span className="mt-0.5 block text-xs text-muted-foreground">
          {def.description ? `${def.description} ` : ""}Allowed range: {k.min} to {k.max}.
        </span>
      </label>
    );
  }
  const hint = HINT[k.type];
  return (
    <label htmlFor={id}>
      {label}
      <input id={id} type="text" spellCheck={false} autoComplete="off" className={fieldClass} value={String(value ?? "")} onChange={(e) => onChange(e.target.value)} />
      {def.description ? <span className="mt-0.5 block text-xs text-muted-foreground">{def.description}</span> : null}
      {hint ? <span className="mt-0.5 block text-xs text-muted-foreground">{hint}</span> : null}
    </label>
  );
}
