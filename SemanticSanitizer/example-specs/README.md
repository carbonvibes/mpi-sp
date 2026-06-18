# Example YAML Specifications

SemanticSanitizer eases development of new sanitizers by providing the
[bootstrap](../bootstrap/) toolkit, which generates the BPF C code and
the Go binding code for new sanitizers based on a specification.

While the example sanitizers distributed with SemanticSanitizer are
kept in the repository as post-codegen BPF C / Go code, this directory
holds the corresponding YAML specifications that can be used to
generate them.

The bootstrap tool follows a one-size-fits-all approach, so
sophisticated checks sometimes require post-codegen modifications of
the resulting BPF C / Go code. For the example sanitizers for which
this is the case, the YAML specifications in this directory contain
a comment indicating this fact.

In all cases, the actual sanitization logic needs to be implemented
in the BPF C code.
