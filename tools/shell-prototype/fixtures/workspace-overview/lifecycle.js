import { View, div } from "gpui-kit";
import { lifecycle_ping } from "conduit-probe";
export default class LifecycleProbe extends View {
  init(_props, cx) {
    cx.spawn(async (cx) => {
      await cx.sleep(1000);
      lifecycle_ping();
    }, { owner: null });
  }
  render() { return div().child("Owner-less task cleanup probe"); }
}
