import { ColumnLineageHost } from "./lineage-trace-host";
import { LineageView } from "./lineage-view";

/** The governance bento's lineage picture: the still page markup, made live after the first paint (WEB-5 T3). */
export function ColumnLineage() {
  return (
    <ColumnLineageHost>
      <LineageView path={0} />
    </ColumnLineageHost>
  );
}
