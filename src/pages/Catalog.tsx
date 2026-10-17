import { useState } from "react";
import { Link } from "react-router-dom";
import { Badge } from "@/components/ui/badge";
import { Card } from "@/components/ui/card";
import { AsyncView } from "@/components/AsyncView";
import { useAsync } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";
import type { Category } from "@/domain/types";

const CATEGORIES: { value: Category; label: string }[] = [
  { value: "reconnaissance", label: "Reconnaissance" },
  { value: "web_application", label: "Web application" },
  { value: "network", label: "Network" },
  { value: "passwords", label: "Passwords" },
  { value: "vulnerability", label: "Vulnerability" },
  { value: "forensics", label: "Forensics" },
  { value: "utilities", label: "Utilities" },
];

export function Catalog() {
  const backend = useBackend();
  const [text, setText] = useState("");
  const [category, setCategory] = useState<Category | "">("");
  const [state, retry] = useAsync(
    () => backend.listTools({ text: text || null, category: category || null }),
    [backend, text, category],
  );

  return (
    <div className="mx-auto max-w-4xl space-y-6">
      <h1 className="text-2xl font-semibold">Tool catalog</h1>
      <div className="flex gap-3">
        <input
          aria-label="Search tools"
          placeholder="Search tools…"
          value={text}
          onChange={(e) => setText(e.target.value)}
          className="h-9 flex-1 rounded-md border border-border bg-card px-3 text-sm"
        />
        <select
          aria-label="Filter by category"
          value={category}
          onChange={(e) => setCategory(e.target.value as Category | "")}
          className="h-9 rounded-md border border-border bg-card px-3 text-sm"
        >
          <option value="">All categories</option>
          {CATEGORIES.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </select>
      </div>
      <AsyncView state={state} retry={retry}>
        {(entries) =>
          entries.length === 0 ? (
            <p className="text-muted-foreground">No tools match your search.</p>
          ) : (
            <ul className="grid gap-4 md:grid-cols-2">
              {entries.map(({ manifest: m }) => (
                <li key={m.id}>
                  <Link to={`/catalog/${m.id}`} className="block">
                    <Card className="h-full hover:bg-muted">
                      <div className="flex items-center justify-between">
                        <h2 className="font-semibold">{m.name}</h2>
                        <Badge>{m.category.replace("_", " ")}</Badge>
                      </div>
                      <p className="mt-2 text-sm text-muted-foreground">{m.summary}</p>
                    </Card>
                  </Link>
                </li>
              ))}
            </ul>
          )
        }
      </AsyncView>
    </div>
  );
}
