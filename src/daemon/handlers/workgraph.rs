//! Work-Graph 可视化快照 handler（纯只读）。

use super::*;

/// GET /api/v1/workgraph - 枚举 daemon 持有的全部 Work-Graph 运行时，返回
/// 每个会话的只读快照（选中计划、节点链、分解单元、审计尾部），供 Web
/// WorkGraphPanel 渲染。会话枚举由受信 store 完成，不接受客户端传入的
/// session id。
pub async fn get_work_graph(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<crate::org_graph::WorkGraphSnapshotResponse>, (StatusCode, String)> {
    match state.work_graph_runtime_store.snapshots() {
        Ok(response) => Ok(Json(response)),
        Err(e) => {
            tracing::warn!(error = %e, "get_work_graph failed");
            Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
    }
}
