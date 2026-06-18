package structparser

import (
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"strings"
)

// packageTemplate is a minimal Go package template used for parsing.
const packageTemplate = "package binding\n\n"

// ToCStruct converts the given Go struct definition into a C struct definition.
func ToCStruct(structdef string) (string, error) {
	f, err := parser.ParseFile(token.NewFileSet(), "",
		packageTemplate+structdef, parser.SkipObjectResolution)
	if err != nil {
		return "", fmt.Errorf("parse struct definition: %w", err)
	}

	if len(f.Decls) != 1 {
		return "", fmt.Errorf("expected exactly one declaration, got %d", len(f.Decls))
	}

	genDecl, ok := f.Decls[0].(*ast.GenDecl)
	if !ok {
		return "", fmt.Errorf("expected type declaration, got %T", f.Decls[0])
	}

	if genDecl.Tok != token.TYPE {
		return "", fmt.Errorf("expected type declaration, got %s", genDecl.Tok)
	}

	if len(genDecl.Specs) != 1 {
		return "", fmt.Errorf("expected exactly one type spec, got %d", len(genDecl.Specs))
	}

	typeSpec, ok := genDecl.Specs[0].(*ast.TypeSpec)
	if !ok {
		return "", fmt.Errorf("expected type spec, got %T", genDecl.Specs[0])
	}

	structType, ok := typeSpec.Type.(*ast.StructType)
	if !ok {
		return "", fmt.Errorf("expected struct type, got %T", typeSpec.Type)
	}

	structName := strings.ToLower(typeSpec.Name.Name)
	var fields []string

	for _, field := range structType.Fields.List {
		if len(field.Names) != 1 {
			return "", fmt.Errorf("expected exactly one field name, got %d", len(field.Names))
		}

		fieldName := strings.ToLower(field.Names[0].Name)
		cType, arraySize, err := goTypeToCType(field.Type)
		if err != nil {
			return "", fmt.Errorf("convert field %s: %w", fieldName, err)
		}

		if arraySize != "" {
			fields = append(fields, fmt.Sprintf("%s %s[%s]", cType, fieldName, arraySize))
		} else {
			fields = append(fields, fmt.Sprintf("%s %s", cType, fieldName))
		}
	}

	return fmt.Sprintf("struct %s { %s; };", structName, strings.Join(fields, "; ")), nil
}

func goTypeToCType(typ ast.Expr) (string, string, error) {
	ident, ok := typ.(*ast.Ident)
	if !ok {
		return "", "", fmt.Errorf("unsupported type: %T", typ)
	}

	switch ident.Name {
	case "string":
		return "char", "64", nil
	case "uint32":
		return "__u32", "", nil
	default:
		return "", "", fmt.Errorf("unsupported type: %s", ident.Name)
	}
}
