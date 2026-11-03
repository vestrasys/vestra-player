//! A plugin request's route: answers, refusals, warnings and `dispatch`.
use super::*;

impl Host {
    pub(crate) fn answer(&mut self, reply: Reply, method: &str, params: &Value) {
        match self.dispatch(&reply, method, params) {
            Ok(None) => (),
            Ok(Some(value)) => self.respond(reply, Ok(value)),
            Err(error) => {
                if debug_keys() && method == "ui.menu" {
                    diag!("keys: ui.menu refused {}: {}", error.code, error.message);
                }
                self.respond(reply, Err(error))
            }
        }
    }
    /// Mount requests held while the built-ins start, answered in order once they have.
    pub(crate) fn release_held_mounts(&mut self) {
        if self.held_mounts.is_empty() {
            return;
        }
        let starting = self.builtins_starting();
        for (reply, method, params) in self.held_mounts.release(starting) {
            if self.plugins.contains_key(&reply.owner) {
                self.answer(reply, &method, &params);
            }
        }
    }
    /// The manifest id of a running plugin (its owner number otherwise).
    pub(crate) fn plugin_id(&self, owner: u64) -> String {
        self.plugins
            .get(&owner)
            .map_or_else(|| owner.to_string(), |p| p.manifest.id.clone())
    }
    /// Refuse what another plugin holds (`RpcError::taken`): recorded for
    /// A sentence of a plugin's (`osd.show`, dialogs) in the interface
    /// language: a template translated with its catalog and formatted with
    /// `args`, or a text value (`i18n::text`).
    pub(crate) fn plugin_sentence(
        &self,
        owner: u64,
        text: &Value,
        args: &Value,
    ) -> std::result::Result<String, RpcError> {
        let texts = self.plugins.get(&owner).map_or(
            i18n::text::Texts {
                catalog: None,
                locale: i18n::locale(),
            },
            process::Plugin::texts,
        );
        texts.sentence(text, args).map_err(RpcError::invalid)
    }
    /// `packages.list`, shown on the OSD and announced as `host.error`.
    pub(crate) fn refuse_taken(&mut self, owner: u64, error: RpcError, text: String) -> RpcError {
        let id = self.plugin_id(owner);
        let mut warning = Value::Object(error.detail.clone().unwrap_or_default());
        if let Some(reason) = warning.as_object_mut().and_then(|w| w.remove("reason")) {
            warning["code"] = reason;
        }
        let warnings = self.warnings.edit().entry(owner).or_default();
        warnings.retain(|(w, _)| w["code"] != warning["code"] || w["slot"] != warning["slot"]);
        warnings.push((warning.clone(), text.clone()));
        diag!("{id}: {}", error.message);
        self.show_osd(&i18n::tr_args("{}: {}", &json!([id, text])));
        warning["plugin"] = id.into();
        warning["message"] = error.message.clone().into();
        self.broadcast("host.error", warning);
        error
    }
    pub(crate) fn clear_warning(&mut self, owner: u64, code: &str) {
        if let Some(warnings) = self.warnings.edit().get_mut(&owner) {
            warnings.retain(|(w, _)| w["code"] != code);
        }
    }
    /// `packages.list` `warnings` of a running plugin.
    pub(crate) fn warnings_of(&self, owner: u64) -> Vec<Value> {
        self.warnings
            .get(&owner)
            .into_iter()
            .flatten()
            .map(|(warning, _)| warning.clone())
            .collect()
    }
    /// The same warnings as text: 「面板未显示：侧栏已被 vestra.playlist 使用」.
    pub(crate) fn warning_text(&self, owner: u64) -> Option<String> {
        let text = self
            .warnings
            .get(&owner)?
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>();
        (!text.is_empty()).then(|| text.join(&i18n::tr("; ")))
    }
    pub(crate) fn respond(
        &mut self,
        reply: Reply,
        mut result: std::result::Result<Value, RpcError>,
    ) {
        // The backend names the holder of a taken resolver by owner number.
        if let Err(error) = &mut result
            && let Some(detail) = error.detail.as_mut()
            && detail.get("reason").and_then(Value::as_str) == Some("resolver-taken")
            && let Some(holder) = detail.remove("holder").and_then(|h| h.as_u64())
        {
            let target = detail
                .remove("target")
                .and_then(|t| t.as_str().map(str::to_owned))
                .unwrap_or_default();
            let id = self.plugin_id(holder);
            detail.insert("owner".into(), id.clone().into());
            detail.insert("target".into(), target.clone().into());
            error.message = i18n::tr_args("{} is resolved by {}", &json!([target, id]));
            let text = i18n::tr_args("{} not taken over: resolved by {}", &json!([target, id]));
            let refusal = error.clone();
            result = Err(self.refuse_taken(reply.owner, refusal, text));
        }
        let activation_ui = self
            .supervisor
            .activation_ui_take(Pid(reply.owner), reply.id);
        if activation_ui {
            let outcome = activation_outcome(&result);
            let now = self.now_ms();
            if let Err(error) =
                self.supervisor
                    .activation_ui_finish(Pid(reply.owner), reply.id, outcome, now)
            {
                diag!("activation UI completion: {error}");
            }
        }
        let message = match result {
            Ok(value) => json!({"id":reply.id,"result":value}),
            Err(error) => json!({"id":reply.id,"error":error}),
        };
        self.send(reply.owner, message);
        // Its answer first, then what waited for its UI (T9).
        if activation_ui {
            self.ui_finished(reply.owner);
        }
    }
    /// One request of a plugin's: its route (`capabilities::lookup`), the
    /// permission check, text resolution, then the capability's `handle`.
    pub(crate) fn dispatch(
        &mut self,
        reply: &Reply,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Option<Value>, RpcError> {
        let plugin = self
            .plugins
            .get(&reply.owner)
            .ok_or_else(|| RpcError::new("disabled", "plugin no longer running"))?;
        let route = capabilities::lookup(method)
            .ok_or_else(|| RpcError::new("unsupported", "unknown method"))?;
        let capability = route.needs(params, |name| plugin.allows(name));
        if capability.is_some_and(|capability| !plugin.allows(capability)) {
            return Err(RpcError::new(
                "capability",
                "capability must be declared in manifest",
            ));
        }
        // Text values (i18n.md §7) in property values, timeline titles,
        // subtitle track titles and resolver failures become the strings
        // they show, in this plugin's catalog, before anything reads them (so
        // the value and length limits count what is shown).
        let resolved;
        let params = if (matches!(
            method,
            "ui.open"
                | "ui.set"
                | "ui.mount"
                | "ui.setMount"
                | "settings.register"
                | "settings.set"
                | "timeline.set"
                | "subtitles.commit"
        ) && i18n::text::contains(params))
            || (method == "sourceResolvers.respond" && i18n::text::is_text(&params["error"]))
        {
            let mut copy = params.clone();
            plugin
                .texts()
                .resolve(&mut copy)
                .map_err(RpcError::invalid)?;
            resolved = copy;
            &resolved
        } else {
            params
        };
        route.handle(self, reply, method, params)
    }
}
