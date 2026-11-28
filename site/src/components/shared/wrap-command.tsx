import * as React from "react";

/**
 * A shell command that may wrap on a phone (owner QA 2026-11-25: nothing scrolls sideways there).
 * It breaks only at spaces and after a `/`, never inside a word or at a hyphen, so
 * `git clone https://github.com/RantAI-dev/RantAI-Lakehouse.git` wraps as
 * `git clone https://github.com/` and `RantAI-dev/RantAI-Lakehouse.git`. The text is unchanged
 * (selecting or copying it gives the command); where it fits on one line it shows as before.
 * The callers render it below 1024 px only, next to the plain text for desktop: Chrome breaks at a
 * <wbr> even under `white-space: nowrap`, and the pieces' spans move a glyph by a sub-pixel.
 */
export function WrapCommand({ text }: Readonly<{ text: string }>) {
  return (
    <>
      {text.split(/(\s+)/).map((word, i) =>
        /^\s+$/.test(word) ? (
          <React.Fragment key={i}>{word}</React.Fragment>
        ) : (
          word
            .split(/(?<=\/)(?!\/)/)
            .map((piece, j) => (
              <React.Fragment key={`${i}-${j}`}>
                {j > 0 && <wbr />}
                <span className="whitespace-nowrap">{piece}</span>
              </React.Fragment>
            ))
        ),
      )}
    </>
  );
}
