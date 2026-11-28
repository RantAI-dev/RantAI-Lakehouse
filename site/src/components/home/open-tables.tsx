import { OneTableManyReadersHost } from "./open-tables-host";
import { OneTableView, SETTLED, type Copy } from "./open-tables-view";

/** The Open tables picture: the settled page markup, made live after the first paint (WEB-5 T3). */
export function OneTableManyReaders({ copy }: Readonly<{ copy: Copy }>) {
  return (
    <OneTableManyReadersHost copy={copy}>
      <OneTableView copy={copy} state={SETTLED} />
    </OneTableManyReadersHost>
  );
}
