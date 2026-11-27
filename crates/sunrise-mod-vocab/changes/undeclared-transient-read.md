# A sampled transient target the pass does not declare in its reads is refused

A draw or compute dispatch that samples a transient attachment the pass does not declare with `.reads(FrameTarget::Transient(id))` is now refused at submit, and nothing is recorded. Declare the read on a consumer pass that does not write the transient; if one pass both writes and reads it, split producer and consumer. The error types and hook signatures are unchanged. Which error each case returns is documented on `PipelineDrawError::InvalidBinding`.
