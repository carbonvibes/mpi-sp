package structparser

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestToCStruct(t *testing.T) {
	testCases := map[string]struct {
		goStructDef     string
		expectedCStruct string
		wantErr         bool
	}{
		"valid": {
			goStructDef:     "type Foo struct { Bar string; Baz uint32 }",
			expectedCStruct: `struct foo { char bar[64]; __u32 baz; };`,
		},
		"unsupported type": {
			goStructDef: "type Foo struct { Bar float64 }",
			wantErr:     true,
		},
		"more than one struct": {
			goStructDef: "type Foo struct { Bar string }\ntype Baz struct { Qux int }",
			wantErr:     true,
		},
		"incomplete struct": {
			goStructDef: "type Foo struct { Bar s",
			wantErr:     true,
		},
		"int as type": {
			goStructDef: "type Foo struct { Bar 123 }",
			wantErr:     true,
		},
	}

	for name, tc := range testCases {
		t.Run(name, func(t *testing.T) {
			require := require.New(t)

			cStruct, err := ToCStruct(tc.goStructDef)
			if tc.wantErr {
				require.Error(err)
			} else {
				require.NoError(err)
				require.Equal(tc.expectedCStruct, cStruct)
			}
		})
	}
}
