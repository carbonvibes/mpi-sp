package spec

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestValidate(t *testing.T) {
	testCases := map[string]struct {
		spec    *SanitizerSpec
		wantErr bool
	}{
		"valid": {
			spec: func() *SanitizerSpec {
				s := New()
				s.Name = "valid_name"
				return s
			}(),
		},
		"invalid name": {
			spec: func() *SanitizerSpec {
				s := New()
				s.Name = "Invalid_Name"
				return s
			}(),
			wantErr: true,
		},
		"empty binding struct definition": {
			spec: func() *SanitizerSpec {
				s := New()
				s.Name = "valid_name"
				s.BindingStructs = []BindingStruct{
					{Definition: ""},
				}
				return s
			}(),
			wantErr: true,
		},
		"invalid tracepoint name": {
			spec: func() *SanitizerSpec {
				s := New()
				s.Name = "valid_name"
				s.Tracepoints["Invalid-Name"] = TracepointSpec{
					Group: "raw_syscalls",
					Name:  "sys_enter",
				}
				return s
			}(),
			wantErr: true,
		},
		"empty tracepoint group": {
			spec: func() *SanitizerSpec {
				s := New()
				s.Name = "valid_name"
				s.Tracepoints["valid_name"] = TracepointSpec{
					Group: "",
					Name:  "sys_enter",
				}
				return s
			}(),
			wantErr: true,
		},
	}

	for name, tc := range testCases {
		t.Run(name, func(t *testing.T) {
			require := require.New(t)

			err := tc.spec.Validate()
			if tc.wantErr {
				require.Error(err)
			} else {
				require.NoError(err)
			}
		})
	}
}
