use super::stack_frame_list::{StackFrameId, StackFrameList, StackFrameListEvent};
use anyhow::{anyhow, Result};
use collections::IndexMap;
use dap::{ScopePresentationHint};
use editor::{actions::SelectAll, Editor, EditorEvent};
use gpui::{
    actions, anchored, deferred, list, AnyElement, ClipboardItem, Context, DismissEvent, Entity,
    FocusHandle, Focusable, Hsla, ListOffset, ListState, MouseDownEvent, Point, Subscription, Task,
};
use menu::{Confirm, SelectFirst, SelectLast, SelectNext, SelectPrev};
use project::debugger::session::{self, Scope, Session, Variable, VariableListContainer};
use rpc::proto::{
    self, DebuggerScopeVariableIndex, DebuggerVariableContainer, VariableListScopes,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};
use sum_tree::{Dimension, Item, SumTree, Summary};
use ui::{prelude::*, ContextMenu, ListItem};
use util::{debug_panic, ResultExt};

actions!(variable_list, [ExpandSelectedEntry, CollapseSelectedEntry]);

struct Variable {
    dap: dap::Variable,
    depth: usize,
    // If none, the children are collapsed
    is_expanded: bool,
    children: Option<Vec<Variable>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetVariableState {
    name: String,
    scope: Scope,
    value: String,
    stack_frame_id: u64,
    evaluate_name: Option<String>,
    parent_variables_reference: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum OpenEntry {
    Scope {
        name: String,
    },
    Variable {
        scope_name: String,
        name: String,
        depth: usize,
    },
}


struct ScopeState {
    variables: Vec<Variable>,
    is_expanded: bool,
}





type ScopeId = u64;

enum VariableListEntry {
    Scope(ScopeId),
    Variable(Variable),
}
type IsToggled = bool;
pub struct VariableList {
    list: ListState,
    focus_handle: FocusHandle,
    open_entries: Vec<OpenEntry>,
    session: Entity<Session>,
    _subscriptions: Vec<Subscription>,
    set_variable_editor: Entity<Editor>,
    selection: Option<VariableListEntry>,
    scopes: HashMap<StackFrameId, IndexMap<u64, ScopeState>>,
    set_variable_state: Option<SetVariableState>,
    fetch_variables_task: Option<Task<Result<()>>>,
    open_context_menu: Option<(Entity<ContextMenu>, Point<Pixels>, Subscription)>,
}


impl VariableList {
    pub fn new(
        session: Entity<Session>,
        stack_frame_list: Entity<StackFrameList>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak_variable_list = cx.weak_entity();
        let focus_handle = cx.focus_handle();

        let list = ListState::new(
            0,
            gpui::ListAlignment::Top,
            px(1000.),
            move |ix, _window, cx| {
                weak_variable_list
                    .upgrade()
                    .map(|var_list| var_list.update(cx, |this, cx| this.render_entry(ix, cx)))
                    .unwrap_or(div().into_any())
            },
        );

        let set_variable_editor = cx.new(|cx| Editor::single_line(window, cx));

        cx.subscribe(
            &set_variable_editor,
            |this: &mut Self, _, event: &EditorEvent, cx| {
                if *event == EditorEvent::Blurred {
                    this.cancel_set_variable_value(cx);
                }
            },
        )
        .detach();

        let _subscriptions =
            vec![cx.subscribe(&stack_frame_list, Self::handle_stack_frame_list_events)];

        Self {
            list,
            session,
            focus_handle,
            _subscriptions,
            selection: None,
            set_variable_editor,
            open_context_menu: None,
            set_variable_state: None,
            fetch_variables_task: None,
            scopes: Default::default(),
            open_entries: Default::default(),
        }
    }

    pub fn variable_list(
        &mut self,
        stack_frame_id: u64,
        cx: &mut Context<Self>,
    ) -> Vec<VariableListContainer> {
        self.scopes( stack_frame_id, cx)
            .iter()
            .cloned()
            .flat_map(|scope| {

            })
            .collect()
    }


    fn scopes_to_variable_list(&mut self, stack_frame_id: &StackFrameId, cx: &mut Context<Self>) -> Vec<VariableListEntry> {
        let mut ret = vec![];

        let Some(scopes) = self.scopes.get_mut(stack_frame_id) else {
            return ret;
        };
        for (scope, scope_state) in scopes {

            if scope_state.is_expanded && scope_state.variables.is_empty() {
                scope_state.variables = self.session.update(cx, |this, cx| {
                    this.variables(
                        scope.dap.variables_reference,
                        cx,
                        )});
            }

            let mut stack = vec![scope.dap.variables_reference];
            let head = VariableListContainer::Scope(scope);
            let mut variables = vec![head];

            while let Some(reference) = stack.pop() {
                if let Some(children) = self.variables.get(&reference) {
                    for variable in children {
                        if variable.toggled_state == ToggledState::Toggled {
                            stack.push(variable.dap.variables_reference);
                        }

                        variables.push(VariableListContainer::Variable(variable.clone()));
                    }
                }
            }

            variables
        }
        ret
    }

    fn handle_stack_frame_list_events(
        &mut self,
        _: Entity<StackFrameList>,
        event: &StackFrameListEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            StackFrameListEvent::SelectedStackFrameChanged(stack_frame_id) => {
                self.handle_selected_stack_frame_changed(*stack_frame_id, cx);
            }
            StackFrameListEvent::StackFramesUpdated => {
                self.entries.clear();
                self.variables.clear();
                self.scopes.clear();
            }
        }
    }

    fn handle_selected_stack_frame_changed(
        &mut self,
        stack_frame_id: StackFrameId,
        cx: &mut Context<Self>,
    ) {
        // if self.scopes.contains_key(&stack_frame_id) {
        //     return self.build_entries(true, cx);
        // }

        // self.fetch_variables_task = Some(cx.spawn(|this, mut cx| async move {
        //     let task = this.update(&mut cx, |variable_list, cx| {
        //         variable_list.fetch_variables_for_stack_frame(stack_frame_id, cx)
        //     })?;

        //     let (scopes, variables) = task.await?;

        //     this.update(&mut cx, |variable_list, cx| {
        //         variable_list.scopes.insert(stack_frame_id, scopes);

        //         for (scope_id, variables) in variables.into_iter() {
        //             let mut variable_index = ScopeVariableIndex::new();
        //             variable_index.add_variables(scope_id, variables);

        //             variable_list
        //                 .variables
        //                 .insert((stack_frame_id, scope_id), variable_index);
        //         }

        //         variable_list.build_entries(true, cx);

        //         variable_list.fetch_variables_task.take();
        //     })
        // }));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn scopes(&self) -> &HashMap<StackFrameId, Vec<Scope>> {
        &self.scopes
    }


    #[cfg(any(test, feature = "test-support"))]
    pub fn entries(&self) -> &HashMap<StackFrameId, Vec<VariableListEntry>> {
        &self.entries
    }

    pub fn variables_by_scope(
        &self,
        stack_frame_id: StackFrameId,
        scope_id: ScopeId,
    ) -> Option<&ScopeVariableIndex> {
        self.variables.get(&(stack_frame_id, scope_id))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn variables_by_stack_frame_id(
        &self,
        stack_frame_id: StackFrameId,
    ) -> Vec<VariableContainer> {
        self.variables
            .range((stack_frame_id, u64::MIN)..(stack_frame_id, u64::MAX))
            .flat_map(|(_, containers)| containers.variables.iter().cloned())
            .collect()
    }

    pub fn completion_variables(&self, cx: &mut Context<Self>) -> Vec<VariableContainer> {
        let stack_frame_id = self
            .stack_frame_list
            .update(cx, |this, cx| this.get_main_stack_frame_id(cx));

        self.variables
            .range((stack_frame_id, u64::MIN)..(stack_frame_id, u64::MAX))
            .flat_map(|(_, containers)| containers.variables.iter().cloned())
            .collect()
    }

    fn render_entry(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let stack_frame_id = self.stack_frame_list.read(cx).current_stack_frame_id();
        let Some(thread_id) = self.stack_frame_list.read(cx).current_thread_id() else {
            return div().into_any_element();
        };

        let entries = self.session.update(cx, |session, cx| {
            session.variable_list(thread_id, stack_frame_id, cx)
        });

        let Some(entry) = entries.get(ix) else {
            debug_panic!("Trying to render entry in variable list that has an out of bounds index");
            return div().into_any_element();
        };

        let entry = &entries[ix];
        match entry {
            session::VariableListContainer::Scope(scope) => self.render_scope(scope, false, cx), // todo(debugger) pass a valid value for is selected
            session::VariableListContainer::Variable(variable) => {
                self.render_variable(variable, false, cx)
            }
        }
    }

    pub fn toggle_variable(
        &mut self,
        scope: &Scope,
        variable: &Variable,
        depth: usize,
        cx: &mut Context<Self>,
    ) {
        let stack_frame_id = self.stack_frame_list.read(cx).current_stack_frame_id();
        let scope_id = scope.variables_reference;

        let Some(variable_index) = self.variables_by_scope(stack_frame_id, scope_id) else {
            return;
        };

        let entry_id = OpenEntry::Variable {
            depth: 1u8,
            name: variable.dap.name.clone(),
            scope_name: scope.name.clone(),
        };

        let has_children = variable.dap.variables_reference > 0;
        let disclosed = has_children.then(|| self.open_entries.binary_search(&entry_id).is_ok());

        // if we already opened the variable/we already fetched it
        // we can just toggle it because we already have the nested variable
        if disclosed.unwrap_or(true) || variable_index.fetched(&variable.dap.variables_reference) {
            return self.toggle_entry(&entry_id, cx);
        }

        // let fetch_variables_task = self.dap_store.update(cx, |store, cx| {
        //     let thread_id = self.stack_frame_list.read(cx).thread_id();
        //     store.variables(
        //         &self.client_id,
        //         thread_id,
        //         stack_frame_id,
        //         scope_id,
        //         self.session.read(cx).id(),
        //         variable.variables_reference,
        //         cx,
        //     )
        // });
        let fetch_variables_task = Task::ready(anyhow::Result::Err(anyhow!(
            "Toggling variables isn't supported yet (dued to refactor)"
        )));

        let container_reference = variable.dap.variables_reference;
        let entry_id = entry_id.clone();

        self.fetch_variables_task = Some(cx.spawn(|this, mut cx| async move {
            let new_variables: Vec<Variable> = fetch_variables_task.await?;

            this.update(&mut cx, |this, cx| {
                let Some(index) = this.variables.get_mut(&(stack_frame_id, scope_id)) else {
                    return;
                };

                index.add_variables(
                    container_reference,
                    new_variables
                        .into_iter()
                        .map(|variable| VariableContainer {
                            variable,
                            depth: depth + 1,
                            container_reference,
                        })
                        .collect::<Vec<_>>(),
                );

                this.toggle_entry(&entry_id, cx);
            })
        }))
    }

    pub fn toggle_entry(&mut self, entry_id: &OpenEntry, cx: &mut Context<Self>) {
        match self.open_entries.binary_search(&entry_id) {
            Ok(ix) => {
                self.open_entries.remove(ix);
            }
            Err(ix) => {
                self.open_entries.insert(ix, entry_id.clone());
            }
        };
    }

    fn fetch_nested_variables(
        &self,
        scope: &Scope,
        container_reference: u64,
        depth: usize,
        open_entries: &Vec<OpenEntry>,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<VariableContainer>>> {
        Task::ready(Ok(vec![]))
    }

    fn deploy_variable_context_menu(
        &mut self,
        parent_variables_reference: u64,
        scope: &Scope,
        variable: &Variable,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let caps = self.session.read(cx).capabilities();
        let (support_set_variable, support_clipboard_context) = (
            caps.supports_set_variable.unwrap_or_default(),
            caps.supports_clipboard_context.unwrap_or_default(),
        );

        let this = cx.entity();

        let context_menu = ContextMenu::build(window, cx, |menu, window, _cx| {
            menu.entry("Copy name", None, {
                let variable_name = variable.dap.name.clone();
                move |_window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(variable_name.clone()))
                }
            })
            .entry("Copy value", None, {
                let source = scope.source.clone();
                let variable_value = variable.dap.value.clone();
                let variable_name = variable.dap.name.clone();
                let evaluate_name = variable.dap.evaluate_name.clone();

                window.handler_for(&this.clone(), move |this, _window, cx| {
                    if support_clipboard_context {
                        this.session.update(cx, |state, cx| {
                            state.evaluate(
                                evaluate_name.clone().unwrap_or(variable_name.clone()),
                                Some(dap::EvaluateArgumentsContext::Clipboard),
                                Some(this.stack_frame_list.read(cx).current_stack_frame_id()),
                                source.clone(),
                                cx,
                            );
                        });
                        // TODO(debugger): make this work again:
                        // cx.write_to_clipboard(ClipboardItem::new_string(response.result));
                    } else {
                        cx.write_to_clipboard(ClipboardItem::new_string(variable_value.clone()))
                    }
                })
            })
            .when_some(
                variable.dap.memory_reference.clone(),
                |menu, memory_reference| {
                    menu.entry(
                        "Copy memory reference",
                        None,
                        window.handler_for(&this, move |_, _window, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(
                                memory_reference.clone(),
                            ))
                        }),
                    )
                },
            )
            .when(support_set_variable, |menu| {
                let variable = variable.clone();
                let scope = scope.clone();

                menu.entry(
                    "Set value",
                    None,
                    window.handler_for(&this, move |this, window, cx| {
                        this.set_variable_state = Some(SetVariableState {
                            parent_variables_reference,
                            name: variable.dap.name.clone(),
                            scope: scope.clone(),
                            evaluate_name: variable.dap.evaluate_name.clone(),
                            value: variable.dap.value.clone(),
                            stack_frame_id: this.stack_frame_list.read(cx).current_stack_frame_id(),
                        });

                        this.set_variable_editor.update(cx, |editor, cx| {
                            editor.set_text(variable.dap.value.clone(), window, cx);
                            editor.select_all(&SelectAll, window, cx);
                            window.focus(&editor.focus_handle(cx))
                        });

                        // this.build_entries(false, cx);
                    }),
                )
            })
        });

        cx.focus_view(&context_menu, window);
        let subscription = cx.subscribe_in(
            &context_menu,
            window,
            |this, _entity, _event: &DismissEvent, window, cx| {
                if this.open_context_menu.as_ref().is_some_and(|context_menu| {
                    context_menu.0.focus_handle(cx).contains_focused(window, cx)
                }) {
                    cx.focus_self(window);
                }
                this.open_context_menu.take();
                cx.notify();
            },
        );

        self.open_context_menu = Some((context_menu, position, subscription));
    }

    fn cancel_set_variable_value(&mut self, cx: &mut Context<Self>) {
        if self.set_variable_state.take().is_none() {
            return;
        };
    }

    fn set_variable_value(&mut self, _: &Confirm, window: &mut Window, cx: &mut Context<Self>) {
        let new_variable_value = self.set_variable_editor.update(cx, |editor, cx| {
            let new_variable_value = editor.text(cx);

            editor.clear(window, cx);

            new_variable_value
        });

        let Some(set_variable_state) = self.set_variable_state.take() else {
            return;
        };

        if new_variable_value == set_variable_state.value
            || set_variable_state.stack_frame_id
                != self.stack_frame_list.read(cx).current_stack_frame_id()
        {
            return cx.notify();
        }

        self.session.update(cx, |state, cx| {
            state.set_variable_value(
                set_variable_state.parent_variables_reference,
                set_variable_state.name,
                new_variable_value,
                cx,
            );
        });
    }

    fn select_first(&mut self, _: &SelectFirst, _window: &mut Window, cx: &mut Context<Self>) {
        let stack_frame_id = self.stack_frame_list.read(cx).current_stack_frame_id();
        if let Some(entries) = self.entries.get(&stack_frame_id) {
            self.selection = entries.first().cloned();
            cx.notify();
        };
    }

    fn select_last(&mut self, _: &SelectLast, _window: &mut Window, cx: &mut Context<Self>) {
        let stack_frame_id = self.stack_frame_list.read(cx).current_stack_frame_id();
        if let Some(entries) = self.entries.get(&stack_frame_id) {
            self.selection = entries.last().cloned();
            cx.notify();
        };
    }

    // fn select_prev(&mut self, _: &SelectPrev, window: &mut Window, cx: &mut Context<Self>) {
    //     if let Some(selection) = &self.selection {
    //         let stack_frame_id = self.stack_frame_list.read(cx).current_stack_frame_id();
    //         if let Some(entries) = self.entries.get(&stack_frame_id) {
    //             if let Some(ix) = entries.iter().position(|entry| entry == selection) {
    //                 self.selection = entries.get(ix.saturating_sub(1)).cloned();
    //                 cx.notify();
    //             }
    //         }
    //     } else {
    //         self.select_first(&SelectFirst, window, cx);
    //     }
    // }

    fn select_next(&mut self, _: &SelectNext, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(selection) = &self.selection {
            let stack_frame_id = self.stack_frame_list.read(cx).current_stack_frame_id();
            if let Some(entries) = self.entries.get(&stack_frame_id) {
                if let Some(ix) = entries.iter().position(|entry| entry == selection) {
                    self.selection = entries.get(ix + 1).cloned();
                    cx.notify();
                }
            }
        } else {
            self.select_first(&SelectFirst, window, cx);
        }
    }

    fn collapse_selected_entry(
        &mut self,
        _: &CollapseSelectedEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // if let Some(selection) = &self.selection {
        //     match selection {
        //         VariableListEntry::Scope(scope) => {
        //             let entry_id = &OpenEntry::Scope {
        //                 name: scope.name.clone(),
        //             };

        //             if self.open_entries.binary_search(entry_id).is_err() {
        //                 self.select_prev(&SelectPrev, window, cx);
        //             } else {
        //                 self.toggle_entry(entry_id, cx);
        //             }
        //         }
        //         VariableListEntry::Variable {
        //             depth,
        //             variable,
        //             scope,
        //             ..
        //         } => {
        //             let entry_id = &OpenEntry::Variable {
        //                 depth: *depth,
        //                 name: variable.name.clone(),
        //                 scope_name: scope.name.clone(),
        //             };

        //             if self.open_entries.binary_search(entry_id).is_err() {
        //                 self.select_prev(&SelectPrev, window, cx);
        //             } else {
        //                 // todo
        //             }
        //         }
        //         VariableListEntry::SetVariableEditor { .. } => {}
        //     }
        // }
    }

    fn expand_selected_entry(
        &mut self,
        _: &ExpandSelectedEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // todo(debugger) Implement expand_selected_entry
        // if let Some(selection) = &self.selection {
        //     match selection {
        //         VariableListEntry::Scope(scope) => {
        //             let entry_id = &OpenEntry::Scope {
        //                 name: scope.name.clone(),
        //             };

        //             if self.open_entries.binary_search(entry_id).is_ok() {
        //                 self.select_next(&SelectNext, window, cx);
        //             } else {
        //                 self.toggle_entry(entry_id, cx);
        //             }
        //         }
        //         VariableListEntry::Variable {
        //             depth,
        //             variable,
        //             scope,
        //             ..
        //         } => {
        //             let entry_id = &OpenEntry::Variable {
        //                 depth: *depth,
        //                 name: variable.dap.name.clone(),
        //                 scope_name: scope.name.clone(),
        //             };

        //             if self.open_entries.binary_search(entry_id).is_ok() {
        //                 self.select_next(&SelectNext, window, cx);
        //             } else {
        //                 // self.toggle_variable(&scope.clone(), &variable.clone(), *depth, cx);
        //             }
        //         }
        //         VariableListEntry::SetVariableEditor { .. } => {}
        //     }
        // }
    }

    fn render_set_variable_editor(
        &self,
        depth: usize,
        state: &SetVariableState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .h_4()
            .size_full()
            .on_action(cx.listener(Self::set_variable_value))
            .child(
                ListItem::new(SharedString::from(state.name.clone()))
                    .indent_level(depth + 1)
                    .indent_step_size(px(20.))
                    .child(self.set_variable_editor.clone()),
            )
            .into_any_element()
    }

    #[track_caller]
    #[cfg(any(test, feature = "test-support"))]
    pub fn assert_visual_entries(&self, expected: Vec<&str>, cx: &Context<Self>) {
        const INDENT: &'static str = "    ";

        let stack_frame_id = self.stack_frame_list.read(cx).current_stack_frame_id();
        let entries = self.entries.get(&stack_frame_id).unwrap();

        let mut visual_entries = Vec::with_capacity(entries.len());
        for entry in entries {
            let is_selected = Some(entry) == self.selection.as_ref();

            match entry {
                VariableListEntry::Scope(scope) => {
                    let is_expanded = self
                        .open_entries
                        .binary_search(&OpenEntry::Scope {
                            name: scope.name.clone(),
                        })
                        .is_ok();

                    visual_entries.push(format!(
                        "{} {}{}",
                        if is_expanded { "v" } else { ">" },
                        scope.name,
                        if is_selected { " <=== selected" } else { "" }
                    ));
                }
                VariableListEntry::SetVariableEditor { depth, state } => {
                    visual_entries.push(format!(
                        "{}  [EDITOR: {}]{}",
                        INDENT.repeat(*depth),
                        state.name,
                        if is_selected { " <=== selected" } else { "" }
                    ));
                }
                VariableListEntry::Variable {
                    depth,
                    variable,
                    scope,
                    ..
                } => {
                    let is_expanded = self
                        .open_entries
                        .binary_search(&OpenEntry::Variable {
                            depth: *depth,
                            name: variable.name.clone(),
                            scope_name: scope.name.clone(),
                        })
                        .is_ok();

                    visual_entries.push(format!(
                        "{}{} {}{}",
                        INDENT.repeat(*depth),
                        if is_expanded { "v" } else { ">" },
                        variable.name,
                        if is_selected { " <=== selected" } else { "" }
                    ));
                }
            };
        }

        pretty_assertions::assert_eq!(expected, visual_entries);
    }

    #[allow(clippy::too_many_arguments)]
    fn render_variable(
        &self,
        variable: &Variable,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry_id = OpenEntry::Variable {
            depth: variable.depth,
            name: variable.dap.name.clone(),
            scope_name: "Local".into(),
        };
        let disclosed = variable.is_expanded;

        let colors = get_entry_color(cx);
        let bg_hover_color = if !is_selected {
            colors.hover
        } else {
            colors.default
        };
        let border_color = if is_selected {
            colors.marked_active
        } else {
            colors.default
        };

        div()
            .id(SharedString::from(format!(
                "variable-{}-{}",
                variable.dap.name, variable.depth
            )))
            .group("variable_list_entry")
            .border_1()
            .border_r_2()
            .border_color(border_color)
            .h_4()
            .size_full()
            .hover(|style| style.bg(bg_hover_color))
            .on_click(cx.listener({
                // let scope = scope.clone();
                // let variable = variable.clone();
                move |this, _, _window, cx| {
                    // this.selection = Some(VariableListEntry::Variable {
                    //     depth,
                    //     has_children,
                    //     container_reference,
                    //     scope: scope.clone(),
                    //     variable: variable.clone(),
                    // });
                    // cx.notify();
                }
            }))
            .child(
                ListItem::new(SharedString::from(format!(
                    "variable-item-{}-{}",
                    variable.dap.name, variable.depth
                )))
                .selectable(false)
                .indent_level(variable.depth as usize)
                .indent_step_size(px(20.))
                .always_show_disclosure_icon(true)
                .toggle(disclosed)
                .when(
                    variable.dap.variables_reference > 0,
                    |list_item| {
                        list_item.on_toggle(cx.listener({
                            let variable = variable.clone();
                            move |this, _, _window, cx| {
                                this.session.update(cx, |session, cx| {
                                    session.variables(thread_id, stack_frame_id, variables_reference, cx)
                                })
                                this.toggle_variable(&scope, &variable, depth, cx)
                            }
                        }))
                    },
                )
                .on_secondary_mouse_down(cx.listener({
                    // let scope = scope.clone();
                    // let variable = variable.clone();
                    move |this, event: &MouseDownEvent, window, cx| {
                        // todo(debugger): Get this working
                        // this.deploy_variable_context_menu(
                        //     container_reference,
                        //     &scope,
                        //     &variable,
                        //     event.position,
                        //     window,
                        //     cx,
                        // )
                    }
                }))
                .child(
                    h_flex()
                        .gap_1()
                        .text_ui_sm(cx)
                        .child(variable.dap.name.clone())
                        .child(
                            div()
                                .text_ui_xs(cx)
                                .text_color(cx.theme().colors().text_muted)
                                .child(variable.dap.value.replace("\n", " ").clone()),
                        ),
                ),
            )
            .into_any()
    }

    fn render_scope(
        &self,
        scope: &session::Scope,
        is_selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let element_id = scope.dap.variables_reference;

        let entry_id = OpenEntry::Scope {
            name: scope.dap.name.clone(),
        };

        // todo(debugger) set this based on the scope being toggled or not
        let disclosed = true;

        let colors = get_entry_color(cx);
        let bg_hover_color = if !is_selected {
            colors.hover
        } else {
            colors.default
        };
        let border_color = if is_selected {
            colors.marked_active
        } else {
            colors.default
        };

        div()
            .id(element_id as usize)
            .group("variable_list_entry")
            .border_1()
            .border_r_2()
            .border_color(border_color)
            .flex()
            .w_full()
            .h_full()
            .hover(|style| style.bg(bg_hover_color))
            .on_click(cx.listener({
                move |this, _, _window, cx| {
                    cx.notify();
                }
            }))
            .child(
                ListItem::new(SharedString::from(format!(
                    "scope-{}",
                    scope.dap.variables_reference
                )))
                .selectable(false)
                .indent_level(1)
                .indent_step_size(px(20.))
                .always_show_disclosure_icon(true)
                .toggle(disclosed)
                .child(div().text_ui(cx).w_full().child(scope.dap.name.clone())),
            )
            .into_any()
    }
}

impl Focusable for VariableList {
    fn focus_handle(&self, _: &App) -> gpui::FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VariableList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // todo(debugger): We are reconstructing the variable list list state every frame
        // which is very bad!! We should only reconstruct the variable list state when necessary.
        // Will fix soon
        let (stack_frame_id, thread_id) = self.stack_frame_list.read_with(cx, |list, cx| {
            (list.current_stack_frame_id(), list.current_thread_id())
        });
        let len = if let Some(thread_id) = thread_id {
            self.session
                .update(cx, |session, cx| {
                    session.variable_list(thread_id, stack_frame_id, cx)
                })
                .len()
        } else {
            0
        };
        self.list.reset(len);

        div()
            .key_context("VariableList")
            .id("variable-list")
            .group("variable-list")
            .size_full()
            .track_focus(&self.focus_handle(cx))
            .on_action(cx.listener(Self::select_first))
            .on_action(cx.listener(Self::select_last))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::select_next))
            // .on_action(cx.listener(Self::expand_selected_entry))
            // .on_action(cx.listener(Self::collapse_selected_entry))
            .on_action(
                cx.listener(|this, _: &editor::actions::Cancel, _window, cx| {
                    this.cancel_set_variable_value(cx)
                }),
            )
            .child(list(self.list.clone()).gap_1_5().size_full())
            .children(self.open_context_menu.as_ref().map(|(menu, position, _)| {
                deferred(
                    anchored()
                        .position(*position)
                        .anchor(gpui::Corner::TopLeft)
                        .child(menu.clone()),
                )
                .with_priority(1)
            }))
    }
}

struct EntryColors {
    default: Hsla,
    hover: Hsla,
    marked_active: Hsla,
}

fn get_entry_color(cx: &Context<VariableList>) -> EntryColors {
    let colors = cx.theme().colors();

    EntryColors {
        default: colors.panel_background,
        hover: colors.ghost_element_hover,
        marked_active: colors.ghost_element_selected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_initial_variables_to_index() {
        let mut index = ScopeVariableIndex::new();

        assert_eq!(index.variables(), vec![]);
        assert_eq!(index.fetched_ids, HashSet::default());

        let variable1 = VariableContainer {
            variable: Variable {
                name: "First variable".into(),
                value: "First variable".into(),
                type_: None,
                presentation_hint: None,
                evaluate_name: None,
                variables_reference: 0,
                named_variables: None,
                indexed_variables: None,
                memory_reference: None,
            },
            depth: 1,
            container_reference: 1,
        };

        let variable2 = VariableContainer {
            variable: Variable {
                name: "Second variable with child".into(),
                value: "Second variable with child".into(),
                type_: None,
                presentation_hint: None,
                evaluate_name: None,
                variables_reference: 2,
                named_variables: None,
                indexed_variables: None,
                memory_reference: None,
            },
            depth: 1,
            container_reference: 1,
        };

        let variable3 = VariableContainer {
            variable: Variable {
                name: "Third variable".into(),
                value: "Third variable".into(),
                type_: None,
                presentation_hint: None,
                evaluate_name: None,
                variables_reference: 0,
                named_variables: None,
                indexed_variables: None,
                memory_reference: None,
            },
            depth: 1,
            container_reference: 1,
        };

        index.add_variables(
            1,
            vec![variable1.clone(), variable2.clone(), variable3.clone()],
        );

        assert_eq!(
            vec![variable1.clone(), variable2.clone(), variable3.clone()],
            index.variables(),
        );
        assert_eq!(HashSet::from([1]), index.fetched_ids,);
    }

    /// This covers when you click on a variable that has a nested variable
    /// We correctly insert the variables right after the variable you clicked on
    #[test]
    fn test_add_sub_variables_to_index() {
        unimplemented!("This test hasn't been refactored yet")
        // let mut index = ScopeVariableIndex::new();

        // assert_eq!(index.variables(), vec![]);

        // let variable1 = VariableContainer {
        //     variable: Variable {
        //         name: "First variable".into(),
        //         value: "First variable".into(),
        //         type_: None,
        //         presentation_hint: None,
        //         evaluate_name: None,
        //         variables_reference: 0,
        //         named_variables: None,
        //         indexed_variables: None,
        //         memory_reference: None,
        //     },
        //     depth: 1,
        //     container_reference: 1,
        // };

        // let variable2 = VariableContainer {
        //     variable: Variable {
        //         name: "Second variable with child".into(),
        //         value: "Second variable with child".into(),
        //         type_: None,
        //         presentation_hint: None,
        //         evaluate_name: None,
        //         variables_reference: 2,
        //         named_variables: None,
        //         indexed_variables: None,
        //         memory_reference: None,
        //     },
        //     depth: 1,
        //     container_reference: 1,
        // };

        // let variable3 = VariableContainer {
        //     variable: Variable {
        //         name: "Third variable".into(),
        //         value: "Third variable".into(),
        //         type_: None,
        //         presentation_hint: None,
        //         evaluate_name: None,
        //         variables_reference: 0,
        //         named_variables: None,
        //         indexed_variables: None,
        //         memory_reference: None,
        //     },
        //     depth: 1,
        //     container_reference: 1,
        // };

        // index.add_variables(
        //     1,
        //     vec![variable1.clone(), variable2.clone(), variable3.clone()],
        // );

        // assert_eq!(
        //     vec![variable1.clone(), variable2.clone(), variable3.clone()],
        //     index.variables(),
        // );
        // assert_eq!(HashSet::from([1]), index.fetched_ids);

        // let variable4 = VariableContainer {
        //     variable: Variable {
        //         name: "Fourth variable".into(),
        //         value: "Fourth variable".into(),
        //         type_: None,
        //         presentation_hint: None,
        //         evaluate_name: None,
        //         variables_reference: 0,
        //         named_variables: None,
        //         indexed_variables: None,
        //         memory_reference: None,
        //     },
        //     depth: 1,
        //     container_reference: 1,
        // };

        // let variable5 = VariableContainer {
        //     variable: Variable {
        //         name: "Five variable".into(),
        //         value: "Five variable".into(),
        //         type_: None,
        //         presentation_hint: None,
        //         evaluate_name: None,
        //         variables_reference: 0,
        //         named_variables: None,
        //         indexed_variables: None,
        //         memory_reference: None,
        //     },
        //     depth: 1,
        //     container_reference: 1,
        // };

        // index.add_variables(2, vec![variable4.clone(), variable5.clone()]);

        // assert_eq!(
        //     vec![
        //         variable1.clone(),
        //         variable2.clone(),
        //         variable4.clone(),
        //         variable5.clone(),
        //         variable3.clone(),
        //     ],
        //     index.variables(),
        // );
        // assert_eq!(index.fetched_ids, HashSet::from([1, 2]));
    }
}
