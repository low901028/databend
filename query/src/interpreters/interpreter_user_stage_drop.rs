// Copyright 2021 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::sync::Arc;

use async_recursion::async_recursion;
use common_exception::Result;
use common_meta_types::StageType;
use common_planners::DropUserStagePlan;
use common_streams::DataBlockStream;
use common_streams::SendableDataBlockStream;
use common_tracing::tracing;
use common_tracing::tracing::info;
use opendal::ObjectStream;
use opendal::Operator;
use tokio_stream::StreamExt;

use crate::interpreters::Interpreter;
use crate::interpreters::InterpreterPtr;
use crate::sessions::QueryContext;
use crate::storages::stage::StageSource;

#[derive(Debug)]
pub struct DropUserStageInterpreter {
    ctx: Arc<QueryContext>,
    plan: DropUserStagePlan,
}

impl DropUserStageInterpreter {
    pub fn try_create(ctx: Arc<QueryContext>, plan: DropUserStagePlan) -> Result<InterpreterPtr> {
        Ok(Arc::new(DropUserStageInterpreter { ctx, plan }))
    }
}

#[async_trait::async_trait]
impl Interpreter for DropUserStageInterpreter {
    fn name(&self) -> &str {
        "DropUserStageInterpreter"
    }

    #[tracing::instrument(level = "info", skip(self, _input_stream), fields(ctx.id = self.ctx.get_id().as_str()))]
    async fn execute(
        &self,
        _input_stream: Option<SendableDataBlockStream>,
    ) -> Result<SendableDataBlockStream> {
        let plan = self.plan.clone();
        let tenant = self.ctx.get_tenant();
        let user_mgr = self.ctx.get_user_manager();

        if let Ok(stage) = user_mgr.get_stage(&tenant, plan.name.as_str()).await {
            if matches!(&stage.stage_type, StageType::Internal) {
                let op = StageSource::get_op(&self.ctx, &stage).await?;
                let absolute_path = format!("/stage/{}/", stage.stage_name);
                let objects = op.object(&absolute_path).list().await?;
                remove_recursive_objects(objects, op.clone()).await?;
                info!(
                    "drop stage {:?} with all objects removed in stage",
                    stage.stage_name
                );
            }
        }

        user_mgr
            .drop_stage(&tenant, plan.name.as_str(), plan.if_exists)
            .await?;
        Ok(Box::pin(DataBlockStream::create(
            self.plan.schema(),
            None,
            vec![],
        )))
    }
}

#[async_recursion]
async fn remove_recursive_objects(mut objects: Box<dyn ObjectStream>, op: Operator) -> Result<()> {
    while let Some(object) = objects.next().await {
        let path = object?.path();
        if path.ends_with('/') {
            let inner_objects = op.object(&path).list().await?;
            remove_recursive_objects(inner_objects, op.clone()).await?;
        } else {
            op.object(&path).delete().await?
        }
    }
    Ok(())
}
