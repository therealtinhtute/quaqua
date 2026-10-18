use quaqua::grid::types::{Layout, PaneLayout, Rect, Tab};
use quaqua::grid::{HerdrDriver, MoveResult, Snapshot, Target};

/// Fake driver for testing without calling real herdr.
/// Models reality:
/// - Assigns a new pane ID on move.
/// - Creates a tab/layout on NewTab.
/// - Updates current_snapshot with the new pane ID and tab ID.
/// - Updates agent pane_ids (N2).
pub struct FakeHerdrDriver {
    pub current_snapshot: Snapshot,
    pub executed_moves: Vec<(String, Target)>,
    pub executed_renames: Vec<(String, String)>,
    pub next_pane_counter: u64,
    pub next_tab_counter: u64,
}

impl FakeHerdrDriver {
    pub fn new(snapshot: Snapshot) -> Self {
        FakeHerdrDriver {
            current_snapshot: snapshot,
            executed_moves: Vec::new(),
            executed_renames: Vec::new(),
            next_pane_counter: 100,
            next_tab_counter: 100,
        }
    }
}

impl HerdrDriver for FakeHerdrDriver {
    fn snapshot(&mut self) -> Result<Snapshot, String> {
        Ok(self.current_snapshot.clone())
    }

    fn move_pane(&mut self, pane: &str, target: &Target) -> Result<MoveResult, String> {
        self.executed_moves.push((pane.to_string(), target.clone()));
        self.next_pane_counter += 1;
        self.next_tab_counter += 1;
        let new_pane_id = format!("w1:p{}", self.next_pane_counter);

        let tab_id = match target {
            Target::NewTab { label } => {
                let tid = format!("w1:t_{}", self.next_tab_counter);
                self.current_snapshot.tabs.push(Tab {
                    tab_id: tid.clone(),
                    workspace_id: Some("w1".to_string()),
                    label: Some(label.clone()),
                    pane_count: Some(1),
                });
                self.current_snapshot.layouts.push(Layout {
                    tab_id: tid.clone(),
                    workspace_id: Some("w1".to_string()),
                    area: Some(Rect {
                        x: 0,
                        y: 0,
                        width: 200,
                        height: 50,
                    }),
                    panes: vec![PaneLayout {
                        pane_id: new_pane_id.clone(),
                        rect: Some(Rect {
                            x: 0,
                            y: 0,
                            width: 200,
                            height: 50,
                        }),
                    }],
                    zoomed: None,
                });
                tid
            }
            Target::Split { tab, .. } => tab.clone(),
        };

        // Update current_snapshot so subsequent fresh snapshots reflect the moved pane
        if let Some(p) = self
            .current_snapshot
            .panes
            .iter_mut()
            .find(|p| p.pane_id == pane)
        {
            p.pane_id = new_pane_id.clone();
            p.tab_id = Some(tab_id.clone());
        }

        // N2: Update agent pane_id
        for a in &mut self.current_snapshot.agents {
            if a.pane_id.as_deref() == Some(pane) {
                a.pane_id = Some(new_pane_id.clone());
            }
        }

        Ok(MoveResult {
            pane_id: new_pane_id,
            tab_id,
            previous_pane_id: Some(pane.to_string()),
        })
    }

    fn rename_pane(&mut self, pane: &str, label: &str) -> Result<(), String> {
        self.executed_renames
            .push((pane.to_string(), label.to_string()));
        if let Some(p) = self
            .current_snapshot
            .panes
            .iter_mut()
            .find(|p| p.pane_id == pane)
        {
            p.label = Some(label.to_string());
        }
        Ok(())
    }
}
