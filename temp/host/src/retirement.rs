// 实验生命周期：停止中的 worker 仍可完成清理 RPC，避免 Drop 阻塞宿主。
use crate::{backend::Backend, plugins::Plugin, services::Services};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
struct Stopping {
    plugin: Plugin,
    since: Instant,
    waiter: Option<(String, Value)>,
}
#[derive(Default)]
pub struct Retirement {
    items: HashMap<String, Stopping>,
}
impl Retirement {
    pub fn begin(&mut self, mut plugin: Plugin, waiter: Option<(String, Value)>) {
        let _ = plugin.send(json!({"type":"shutdown"}));
        self.items.insert(
            plugin.route.clone(),
            Stopping {
                plugin,
                since: Instant::now(),
                waiter,
            },
        );
    }
    pub fn contains(&self, route: &str) -> bool {
        self.items.contains_key(route)
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn reply(&mut self, route: &str, message: Value) -> Result<(), String> {
        if let Some(item) = self.items.get_mut(route) {
            item.plugin.send(message)?;
        }
        Ok(())
    }
    pub fn finish(&mut self, route: &str) -> Option<(String, Value)> {
        self.items.remove(route).and_then(|item| item.waiter)
    }
    pub fn expired(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|(_, item)| item.since.elapsed() > Duration::from_secs(2))
            .map(|(route, _)| route.clone())
            .collect()
    }
    pub fn cleanup_request(
        &self,
        owner: &str,
        method: &str,
        params: &Value,
        services: &mut Services,
        backend: &mut Backend,
    ) -> Result<Value, String> {
        crate::services::validate_params(method, params)?;
        match method {
            "effects.remove" | "effects.releaseSnapshot" | "sources.close" => services
                .request(owner, method, params, backend)?
                .ok_or("unsupported cleanup method".into()),
            "player.filters.setGaussian" if params["enabled"] == false => services
                .request(owner, method, params, backend)?
                .ok_or("unsupported cleanup method".into()),
            "media.cancel" => match services.request(owner, method, params, backend) {
                Ok(Some(value)) => Ok(value),
                _ => Ok(Value::Null),
            },
            "ui.patch" => Ok(Value::Null),
            _ => Err("plugin is stopping; only cleanup requests are allowed".into()),
        }
    }
}
