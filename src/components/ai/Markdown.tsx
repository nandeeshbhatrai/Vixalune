import { Copy } from "lucide-react";
import { isValidElement, useState, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children);
  return "";
}

function CodeBlock({ children }: { children?: ReactNode }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(textOf(children).replace(/\n$/, ""));
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      /* clipboard unavailable */
    }
  };
  return (
    <div className="relative my-2">
      <button type="button" onClick={() => void copy()} aria-label="Copy code" className="absolute right-2 top-2 inline-flex items-center gap-1 rounded border border-border bg-card px-1.5 py-0.5 text-xs">
        <Copy size={12} aria-hidden /> {copied ? "Copied" : "Copy"}
      </button>
      <pre className="overflow-x-auto rounded-md border border-border bg-muted p-3 pr-16 text-xs">{children}</pre>
    </div>
  );
}

/**
 * Renders model output. Output is untrusted: raw HTML is not rendered, images are not loaded (they could leak data to a
 * remote server), and links are shown as plain text so nothing navigates the app.
 */
export function Markdown({ text }: { text: string }) {
  return (
    <div className="prose-sm max-w-none break-words [&_li]:ml-5 [&_ol]:list-decimal [&_p]:my-2 [&_table]:my-2 [&_td]:border [&_td]:border-border [&_td]:px-2 [&_th]:border [&_th]:border-border [&_th]:px-2 [&_ul]:list-disc [&_code:not(pre_code)]:rounded [&_code:not(pre_code)]:bg-muted [&_code:not(pre_code)]:px-1">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[[rehypeHighlight, { detect: false, ignoreMissing: true }]]}
        skipHtml
        components={{
          a: ({ children, href }) => (
            <span>
              {children}
              {href ? <span className="text-muted-foreground"> ({href})</span> : null}
            </span>
          ),
          img: ({ alt }) => <span className="text-muted-foreground">[image not loaded{alt ? `: ${alt}` : ""}]</span>,
          pre: ({ children }) => <CodeBlock>{children}</CodeBlock>,
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
