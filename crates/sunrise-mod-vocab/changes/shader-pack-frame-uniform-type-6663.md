A shader-pack program that declares a mod frame uniform with an incompatible GLSL type stays
bound. Only the mismatched uniform reads zero, with no conversion. The warning names the program,
the uniform, and both types once per program and uniform. This also applies to previous-frame
aliases declared with `PackUniformDecl::with_previous`.
