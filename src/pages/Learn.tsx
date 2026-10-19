import { AlertTriangle, BookOpen, ExternalLink, FlaskConical, Play } from "lucide-react";
import { Link, useParams } from "react-router-dom";
import { AsyncView } from "@/components/AsyncView";
import { DemoTag } from "@/components/DemoTag";
import { EmptyState } from "@/components/EmptyState";
import { ErrorNotice } from "@/components/ErrorNotice";
import { PageHeader } from "@/components/PageHeader";
import { Button } from "@/components/ui/button";
import { Card, CardTitle } from "@/components/ui/card";
import type { Tutorial } from "@/domain/types";
import { useAction } from "@/hooks/useAction";
import { useAsync } from "@/hooks/useAsync";
import { useBackend } from "@/infra/backendContext";

export function LearnIndex() {
  const backend = useBackend();
  const [state, retry] = useAsync(() => backend.listTutorials(), [backend]);
  return (
    <div className="space-y-4">
      <PageHeader title="Learn" subtitle="A short guided lesson for each tool: what it is for, how to run it safely, and how to read the result." actions={<DemoTag />} />
      <AsyncView state={state} retry={retry} label="Loading tutorials">
        {(list) =>
          list.length === 0 ? (
            <EmptyState icon={BookOpen} title="No tutorials available">
              Tutorials are bundled with Vixalune. If this list is empty, a tutorial file may have been rejected; see Settings.
            </EmptyState>
          ) : (
            <ul className="grid gap-3 sm:grid-cols-2">
              {list.map((t) => (
                <li key={t.tool_id}>
                  <Card className="flex h-full flex-col gap-2">
                    <h2 className="font-semibold">{t.title}</h2>
                    <p className="flex-1 text-muted-foreground">{t.what_it_does}</p>
                    <Link to={`/learn/${t.tool_id}`} className="text-primary hover:underline">
                      Start the tutorial
                    </Link>
                  </Card>
                </li>
              ))}
            </ul>
          )
        }
      </AsyncView>
    </div>
  );
}

function TutorialBody({ t }: { t: Tutorial }) {
  const backend = useBackend();
  const { error, run } = useAction();
  const profilesByName = new Map<string, Tutorial["parameters"]>();
  for (const p of t.parameters) profilesByName.set(p.profile, [...(profilesByName.get(p.profile) ?? []), p]);

  return (
    <div className="space-y-4">
      <div role="note" aria-label="Authorization boundary" className="flex gap-3 rounded-lg border border-warning/40 bg-warning/10 p-3">
        <AlertTriangle size={16} className="mt-0.5 shrink-0 text-warning" aria-hidden />
        <p className="text-sm">
          <strong>Stay inside your authorization. </strong>
          {t.authorization_warning}
        </p>
      </div>

      <Card>
        <CardTitle>What it does</CardTitle>
        <p>{t.what_it_does}</p>
        <h3 className="mb-1 mt-3 text-xs font-semibold uppercase tracking-wider text-muted-foreground">The problem it solves</h3>
        <p>{t.problem_solved}</p>
      </Card>

      <Card>
        <CardTitle>What you need</CardTitle>
        <ul className="list-disc space-y-1 pl-5">
          {t.requirements.map((r) => (
            <li key={r}>{r}</li>
          ))}
        </ul>
      </Card>

      <Card>
        <CardTitle>Your first run</CardTitle>
        <ol className="space-y-3">
          {t.first_run.map((s, i) => (
            <li key={s.title} className="flex gap-3">
              <span aria-hidden className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-muted text-xs font-semibold">
                {i + 1}
              </span>
              <div>
                <p className="font-medium">{s.title}</p>
                <p className="text-muted-foreground">{s.body}</p>
                {s.profile ? (
                  <Link to={`/run/${t.tool_id}?profile=${encodeURIComponent(s.profile)}`} className="mt-1 inline-flex items-center gap-1 text-primary hover:underline">
                    <Play size={12} aria-hidden /> Open this in Run a Tool
                  </Link>
                ) : null}
              </div>
            </li>
          ))}
        </ol>
      </Card>

      <Card>
        <CardTitle>What the options mean</CardTitle>
        <dl className="space-y-3">
          {[...profilesByName].map(([profile, notes]) =>
            notes.map((n) => (
              <div key={`${profile}-${n.name}`}>
                <dt className="font-medium">
                  <span className="font-mono text-xs">{n.name}</span> <span className="text-xs text-muted-foreground">({profile})</span>
                </dt>
                <dd className="text-muted-foreground">{n.explanation}</dd>
              </div>
            )),
          )}
        </dl>
      </Card>

      <Card>
        <CardTitle>Example output and how to read it</CardTitle>
        <pre aria-label="Example output" className="overflow-auto rounded-md bg-background p-3 font-mono text-xs">
          {t.example.output.trim()}
        </pre>
        <ul className="mt-3 list-disc space-y-1 pl-5">
          {t.example.interpretation.map((i) => (
            <li key={i}>{i}</li>
          ))}
        </ul>
      </Card>

      <Card>
        <CardTitle>Common problems</CardTitle>
        <dl className="space-y-3">
          {t.common_errors.map((e) => (
            <div key={e.symptom}>
              <dt className="font-medium">{e.symptom}</dt>
              <dd className="text-muted-foreground">
                <span className="block">Why: {e.cause}</span>
                <span className="block">Fix: {e.fix}</span>
              </dd>
            </div>
          ))}
        </dl>
      </Card>

      <Card>
        <CardTitle>Practice exercise</CardTitle>
        <h3 className="font-semibold">{t.exercise.title}</h3>
        <p className="mt-1 text-muted-foreground">{t.exercise.goal}</p>
        <p className="mt-2 text-sm">
          <FlaskConical size={13} className="mr-1 inline" aria-hidden />
          Uses the {t.exercise.lab === "practice-web" ? "practice web application" : "sample code project"} from{" "}
          <Link to="/labs" className="text-primary hover:underline">
            Learning Labs
          </Link>
          . It runs only on this computer.
        </p>
        <ol className="mt-2 list-decimal space-y-1 pl-5">
          {t.exercise.steps.map((s) => (
            <li key={s}>{s}</li>
          ))}
        </ol>
        <p className="mt-3">
          <span className="font-medium">What you should see: </span>
          {t.exercise.expected}
        </p>
        {t.exercise.stretch ? (
          <p className="mt-2 text-muted-foreground">
            <span className="font-medium">Go further: </span>
            {t.exercise.stretch}
          </p>
        ) : null}
        <Link to={`/run/${t.tool_id}?profile=${encodeURIComponent(t.exercise.profile)}`} className="mt-3 inline-flex items-center gap-1 text-primary hover:underline">
          <Play size={12} aria-hidden /> Start the exercise in Run a Tool
        </Link>
      </Card>

      <Card>
        <CardTitle>Official documentation</CardTitle>
        <ul className="space-y-1">
          {t.links.map((l, i) => (
            <li key={l.url}>
              <button type="button" className="inline-flex items-center gap-1 text-primary hover:underline" onClick={() => void run(() => backend.openTutorialLink(t.tool_id, i))}>
                {l.label} <ExternalLink size={12} aria-hidden />
                <span className="sr-only">(opens in your browser)</span>
              </button>
            </li>
          ))}
        </ul>
        {error ? <ErrorNotice error={error} /> : null}
      </Card>
    </div>
  );
}

export function TutorialPage() {
  const backend = useBackend();
  const { toolId = "" } = useParams();
  const [state, retry] = useAsync(() => backend.getTutorial(toolId), [backend, toolId]);
  return (
    <div className="space-y-4">
      <p>
        <Link to="/learn" className="text-primary hover:underline">
          ← All tutorials
        </Link>
      </p>
      <AsyncView state={state} retry={retry} label="Loading the tutorial">
        {(t) => (
          <>
            <PageHeader title={t.title} actions={<DemoTag />} />
            <TutorialBody t={t} />
            <div>
              <Button asChild>
                <Link to={`/run/${t.tool_id}`}>Run {t.tool_id}</Link>
              </Button>
            </div>
          </>
        )}
      </AsyncView>
    </div>
  );
}
