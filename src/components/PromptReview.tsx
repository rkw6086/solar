import { LoaderCircle, RefreshCw, Sparkles } from "lucide-react";
import type { Orbit, PromptAnalysis } from "../types";
import { orbitPreamble } from "./orbitPrompt";
import "./promptReview.css";

export function PromptReview({ orbit, analysis, selected, busy, error, onSelect, onGenerate }: {
  orbit: Orbit;
  analysis: PromptAnalysis | null;
  selected: "original" | "recommended";
  busy: boolean;
  error: string;
  onSelect: (value: "original" | "recommended") => void;
  onGenerate: () => void;
}) {
  return <section className="prompt-review" aria-label="Review your Orbit's prompt">
    <h3>Choose how your Orbit gets started</h3>
    <p>Compare the prompt built from your answers with Solar’s recommendation. Solar adds current projects, memories, and available tools when the Orbit works.</p>
    <div className="prompt-choice">
      <label className="prompt-choice-heading">
        <input type="radio" name="prompt-choice" value="original" checked={selected === "original"} onChange={() => onSelect("original")} />
        <strong>Your prompt</strong>
      </label>
      <pre tabIndex={0}>{orbitPreamble(orbit)}</pre>
    </div>
    <div className={`prompt-choice ${analysis ? "has-recommendation" : ""}`}>
      <label className="prompt-choice-heading">
        <input type="radio" name="prompt-choice" value="recommended" checked={selected === "recommended"} disabled={!analysis} onChange={() => onSelect("recommended")} />
        <strong>Recommended prompt</strong>
      </label>
      {analysis ? <>
        <pre tabIndex={0}>{analysis.recommendedPrompt}</pre>
        <ul>{analysis.analysis.map((reason, index) => <li key={index}>{reason}</li>)}</ul>
        <p className="prompt-review-source">{analysis.notice}</p>
      </> : <p>{orbit.providerId && orbit.model ? `Reviewing your instructions with ${orbit.model}. Only this draft is sent; no tools are run. You can keep your original prompt at any time.` : "Without a model connection, Solar suggests a local template. No model request is made."}</p>}
      <button className="button button-secondary" type="button" disabled={busy} onClick={onGenerate}>
        {busy ? <LoaderCircle size={15} className="spin" /> : analysis ? <RefreshCw size={15} /> : <Sparkles size={15} />}
        {busy ? "Reviewing prompt…" : analysis ? "Regenerate recommendation" : "Make recommendation"}
      </button>
      {error && <p className="form-error" role="alert">{error}</p>}
    </div>
    <p className="prompt-review-footnote">Your original answers are kept while you compare. The selected prompt is saved only when you create the Orbit.</p>
  </section>;
}
