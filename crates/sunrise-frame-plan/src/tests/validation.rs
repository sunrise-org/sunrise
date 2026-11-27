//! Every input the planner refuses, by its typed error and site; and the shapes it takes as they are.

use super::*;
use crate::model::{InputError, Site};
use crate::plan::plan;

/// Two one-level resources; a frame that clears 0 and draws 0 onto 1; an observer of 1.
fn base() -> Graph {
    looped(
        vec![res(4, 100), res(4, 100)],
        vec![clear(&[0]), kept(&[0], &[1])],
        &[1],
    )
}

fn refused(graph: &Graph) -> InputError {
    plan(graph).expect_err("refused")
}

#[test]
fn resources_need_levels_and_one_to_eight_channels() {
    let mut graph = base();
    graph.resources[1].levels = 0;
    assert_eq!(refused(&graph), InputError::NoLevels(ResourceId(1)));
    for channels in [0, 9] {
        let mut graph = base();
        graph.resources[1].channels = channels;
        assert_eq!(refused(&graph), InputError::Channels(ResourceId(1)));
    }
    // Eight channels, the most a mask holds, are taken.
    let mut graph = base();
    graph.resources[1].channels = 8;
    plan(&graph).expect("eight channels");
}

#[test]
fn an_alias_names_another_unaliased_resource() {
    let mut graph = base();
    graph.resources[1].alias = Some(ResourceId(1));
    assert_eq!(refused(&graph), InputError::Alias(ResourceId(1)));
    let mut graph = base();
    graph.resources.push(res(4, 100));
    graph.resources[1].alias = Some(ResourceId(0));
    graph.resources[2].alias = Some(ResourceId(1));
    assert_eq!(refused(&graph), InputError::Alias(ResourceId(2)));
    let mut graph = base();
    graph.resources[1].alias = Some(ResourceId(7));
    assert_eq!(
        refused(&graph),
        InputError::UnknownResource(Site::Resource(ResourceId(1)))
    );
}

#[test]
fn accesses_name_known_resources_levels_and_channels() {
    let site = Site::Event(id(0, 1));
    let mut graph = base();
    graph.states[0].events[1].reads[0] = whole(5);
    assert_eq!(refused(&graph), InputError::UnknownResource(site));
    let mut graph = base();
    graph.states[0].events[1].reads[0] = Access::Unresolved(vec![ResourceId(5)]);
    assert_eq!(refused(&graph), InputError::UnknownResource(site));
    for levels in [Levels { first: 0, count: 0 }, Levels { first: 1, count: 1 }] {
        let mut graph = base();
        graph.states[0].events[1].reads[0] = Access::Resolved(Region { levels, ..rgba(0) });
        assert_eq!(refused(&graph), InputError::LevelsOutOfRange(site));
    }
    for channels in [0, 0b1_0000] {
        let mut graph = base();
        graph.states[0].events[1].reads[0] = Access::Resolved(Region {
            channels,
            ..rgba(0)
        });
        assert_eq!(refused(&graph), InputError::ChannelsOutOfRange(site));
    }
    let mut graph = base();
    graph.states[0].events[1].reads[0] = Access::Unresolved(Vec::new());
    assert_eq!(refused(&graph), InputError::EmptyUnresolved(site));
}

#[test]
fn removable_never_goes_with_opaque_or_effects() {
    let mut graph = base();
    graph.states[0].events[1].effects = true;
    graph.states[0].events[1].removable = true;
    assert_eq!(refused(&graph), InputError::Removable(id(0, 1)));
    let mut graph = base();
    graph.states[0].events.push(Event {
        removable: true,
        ..opaque(&[0], false)
    });
    assert_eq!(refused(&graph), InputError::Removable(id(0, 2)));
}

#[test]
fn a_clear_reads_nothing_has_no_effects_and_fully_writes_resolved_regions() {
    let mut graph = base();
    graph.states[0].events[0].reads.push(whole(1));
    assert_eq!(refused(&graph), InputError::Clear(id(0, 0)));
    let mut graph = base();
    graph.states[0].events[0].writes[0].cover = Cover::Partial;
    assert_eq!(refused(&graph), InputError::Clear(id(0, 0)));
    let mut graph = base();
    graph.states[0].events[0].writes[0].access = Access::Unresolved(vec![ResourceId(0)]);
    assert_eq!(refused(&graph), InputError::Clear(id(0, 0)));
    // Work done beside a clear is an event of its own.
    let mut graph = base();
    graph.states[0].events[0].effects = true;
    assert_eq!(refused(&graph), InputError::Clear(id(0, 0)));
}

#[test]
fn only_resolved_partial_writes_of_work_are_attachments() {
    let mut graph = base();
    graph.states[0].events[0].writes[0].attachment = true;
    assert_eq!(refused(&graph), InputError::Attachment(id(0, 0)));
    let mut graph = base();
    graph.states[0].events[1].writes[0].access = Access::Unresolved(vec![ResourceId(1)]);
    assert_eq!(refused(&graph), InputError::Attachment(id(0, 1)));
    // An attachment that loads as a clear is a full write, and takes no folded clear.
    let mut graph = base();
    graph.states[0].events[1].writes[0].cover = Cover::Full;
    assert_eq!(refused(&graph), InputError::Attachment(id(0, 1)));
}

#[test]
fn only_full_resolved_non_attachment_writes_of_work_are_clear_only() {
    let mut graph = base();
    graph.states[0].events[0].writes[0].clear = true;
    assert_eq!(refused(&graph), InputError::ClearWrite(id(0, 0)));
    let mut graph = base();
    graph.states[0].events[1].writes[0].clear = true;
    assert_eq!(refused(&graph), InputError::ClearWrite(id(0, 1)));
    let mut graph = base();
    let write = &mut graph.states[0].events[1].writes[0];
    (write.cover, write.attachment, write.clear) = (Cover::Full, false, true);
    assert_eq!(graph.validate(), Ok(()));
    graph.states[0].events[1].writes[0].access = Access::Unresolved(vec![ResourceId(1)]);
    assert_eq!(refused(&graph), InputError::ClearWrite(id(0, 1)));
}

#[test]
fn edges_and_observers_name_known_states_and_positions() {
    let mut graph = base();
    graph.edges[0].to = StateId(1);
    assert_eq!(refused(&graph), InputError::UnknownState(Site::Edge(0)));
    let mut graph = base();
    graph.observers[0].state = StateId(1);
    assert_eq!(refused(&graph), InputError::UnknownState(Site::Observer(0)));
    let mut graph = base();
    graph.observers[0].at = 3;
    assert_eq!(refused(&graph), InputError::ObserverPosition(0));
    let mut graph = base();
    graph.observers[0].region.resource = ResourceId(4);
    assert_eq!(
        refused(&graph),
        InputError::UnknownResource(Site::Observer(0))
    );
}

#[test]
fn empty_and_repeated_shapes_plan_as_they_are() {
    // No states at all, a state with no events and no edges, a repeated edge, an unresolved set naming a resource
    // twice, and an event writing the same region twice: all valid, the last two as their union.
    assert_eq!(plan(&Graph::default()), Ok(crate::plan::Plan::default()));
    let mut graph = base();
    graph.states.push(State::default());
    graph.edges.push(graph.edges[0]);
    graph.states[0].events.push(Event {
        reads: vec![Access::Unresolved(vec![ResourceId(0), ResourceId(0)])],
        ..kept(&[], &[1, 1])
    });
    let planned = plan(&graph).expect("valid");
    // 0 is reached unresolved, so nothing about it is planned; 1 is observed.
    assert_eq!(planned, crate::plan::Plan::default());
    super::interp::assert_sound(&graph, &planned, 3);
}
