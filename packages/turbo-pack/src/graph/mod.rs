use std::collections::HashSet;

use crate::asset::AssetRef;

#[turbo_tasks::value(shared)]
#[derive(PartialEq, Eq)]
pub enum AggregatedGraph {
    Leaf(AssetRef),
    Node {
        depth: usize,
        content: HashSet<AggregatedGraphRef>,
        references: HashSet<AggregatedGraphRef>,
    },
}

#[turbo_tasks::value_impl]
impl AggregatedGraph {
    #[turbo_tasks::constructor(intern)]
    fn leaf(asset: AssetRef) -> Self {
        Self::Leaf(asset)
    }

    fn depth(&self) -> usize {
        match self {
            AggregatedGraph::Leaf(_) => 0,
            AggregatedGraph::Node { depth, .. } => *depth,
        }
    }
}

#[turbo_tasks::value_impl]
impl AggregatedGraphRef {
    pub async fn content(self) -> AggregatedGraphNodeContentRef {
        match &*self.await {
            AggregatedGraph::Leaf(asset) => AggregatedGraphNodeContent::Asset(asset.clone()).into(),
            AggregatedGraph::Node { content, .. } => {
                AggregatedGraphNodeContent::Children(content.clone()).into()
            }
        }
    }

    async fn references(self) -> AggregatedGraphsSetRef {
        match &*self.await {
            AggregatedGraph::Leaf(asset) => {
                let mut refs = HashSet::new();
                for reference in asset.clone().references().await.assets.iter() {
                    let reference = reference.clone().resolve_to_slot().await;
                    if asset != &reference {
                        refs.insert(AggregatedGraphRef::leaf(reference));
                    }
                }
                AggregatedGraphsSet { set: refs }.into()
            }
            AggregatedGraph::Node { references, .. } => {
                let mut set = HashSet::new();
                for item in references
                    .iter()
                    .map(|reference| aggregate_more(reference.clone()))
                    .collect::<Vec<_>>()
                    .into_iter()
                {
                    set.insert(item.resolve_to_slot().await);
                }
                AggregatedGraphsSet { set }.into()
            }
        }
    }

    async fn cost(self) -> AggregationCostRef {
        match &*self.await {
            AggregatedGraph::Leaf(asset) => {
                AggregationCost(asset.clone().references().await.assets.len()).into()
            }
            AggregatedGraph::Node { references, .. } => AggregationCost(references.len()).into(),
        }
    }

    async fn valued_references(self) -> AggregatedGraphsValuedReferencesRef {
        let self_cost = self.clone().cost().await.0;
        let mut inner = HashSet::new();
        let mut outer = HashSet::new();
        let mut references = HashSet::new();
        for (reference, cost) in self
            .references()
            .await
            .set
            .iter()
            .map(|reference| (reference.clone(), reference.clone().cost()))
            .collect::<Vec<_>>()
        {
            let cost = cost.await.0;
            if cost == 0 {
                inner.insert(reference);
            } else if cost > self_cost {
                references.insert(reference);
            } else {
                outer.insert(reference);
            }
        }
        AggregatedGraphsValuedReferences {
            inner,
            outer,
            references,
        }
        .into()
    }
}

#[turbo_tasks::function]
pub async fn aggregate(asset: AssetRef) -> AggregatedGraphRef {
    let mut current = AggregatedGraphRef::leaf(asset);
    loop {
        if current.clone().references().await.set.len() == 0 {
            return current;
        }
        current = aggregate_more(current);
    }
}

#[turbo_tasks::value(value)]
#[derive(Clone, Hash, Debug, PartialEq, Eq)]
struct AggregationDepth(usize);

#[turbo_tasks::value(value)]
#[derive(Clone, Hash, Debug, PartialEq, Eq)]
struct AggregationCost(usize);

#[turbo_tasks::function]
async fn aggregate_more(node: AggregatedGraphRef) -> AggregatedGraphRef {
    let node_data = node.get().await;
    let depth = node_data.depth();
    let mut in_progress = HashSet::new();
    let mut content = HashSet::new();
    let mut references = HashSet::new();
    in_progress.insert(node.clone());

    // only one kind of aggregation can't eliminate cycles with that
    // number of nodes. Alternating the aggregation will get rid of all
    // cycles
    let aggregation = if depth > 0 && depth % 2 == 0 { 3 } else { 2 };
    for _ in 0..aggregation {
        for node in in_progress.iter() {
            content.insert(node.clone());
        }
        let valued_refs = in_progress
            .drain()
            .map(|node| node.clone().valued_references())
            .collect::<Vec<_>>();
        for valued_refs in valued_refs {
            let valued_refs = valued_refs.await;
            for reference in valued_refs.inner.iter() {
                content.insert(reference.clone());
            }
            for reference in valued_refs.references.iter() {
                if content.contains(reference) {
                    continue;
                }
                references.insert(reference.clone());
            }
            for reference in valued_refs.outer.iter() {
                if content.contains(reference) {
                    continue;
                }
                references.remove(&reference);
                in_progress.insert(reference.clone());
            }
        }
    }
    for node in in_progress.into_iter() {
        references.insert(node);
    }
    AggregatedGraph::Node {
        depth: depth + 1,
        content,
        references,
    }
    .into()
}

#[turbo_tasks::value(shared)]
#[derive(PartialEq, Eq)]
struct AggregatedGraphsSet {
    pub set: HashSet<AggregatedGraphRef>,
}

#[turbo_tasks::value(shared)]
#[derive(PartialEq, Eq)]
pub enum AggregatedGraphNodeContent {
    Asset(AssetRef),
    Children(HashSet<AggregatedGraphRef>),
}

#[turbo_tasks::value(shared)]
#[derive(PartialEq, Eq)]
struct AggregatedGraphsValuedReferences {
    pub inner: HashSet<AggregatedGraphRef>,
    pub outer: HashSet<AggregatedGraphRef>,
    pub references: HashSet<AggregatedGraphRef>,
}