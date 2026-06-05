use std::time::Duration;

use crate::state::AppState;
use crate::status::patch_function_runtime_status;

pub(crate) async fn idle_scale_loop(state: AppState, interval: Duration) {
    loop {
        tokio::time::sleep(interval).await;
        for function in state.functions.items() {
            if let Some(snapshot) = state.runtime.scale_idle(&function) {
                patch_function_runtime_status(&state, &function, &snapshot, None).await;
            }
        }
    }
}
