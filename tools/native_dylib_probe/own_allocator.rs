// The phase mod with a `#[global_allocator]` of its own (appended to its source).
#[global_allocator]
static OWN: std::alloc::System = std::alloc::System;
