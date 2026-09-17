// Registers happy-dom's DOM implementation as globals (document, window,
// HTMLElement, ...) for every test process bun's preload list runs this
// in -- @testing-library/react needs a real DOM to render into, and bun
// itself ships none. Preloaded via bunfig.toml, not imported per-test-file,
// so every future test file gets a DOM automatically with no per-file
// boilerplate (judge amendment A2 -- WS3 is the first workstream to need
// this; WS7/WS9 reuse it rather than each registering their own).
import { GlobalRegistrator } from "@happy-dom/global-registrator"

GlobalRegistrator.register()
