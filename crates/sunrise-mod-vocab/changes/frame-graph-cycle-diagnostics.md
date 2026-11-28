`FrameGraphError::Cycle` now lists only passes that belong to a dependency cycle, in
registration order. Acyclic passes downstream of a cycle, including one-way bridges between
separate cycles, are excluded. The graph is still refused, and error types and hook signatures
are unchanged.
