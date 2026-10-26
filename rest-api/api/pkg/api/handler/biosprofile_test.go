// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package handler

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/labstack/echo/v4"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/mock"
	"github.com/stretchr/testify/require"
	tmocks "go.temporal.io/sdk/mocks"
	tp "go.temporal.io/sdk/temporal"
	"google.golang.org/protobuf/encoding/protojson"
	"google.golang.org/protobuf/proto"
	"google.golang.org/protobuf/types/known/timestamppb"

	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/handler/util/common"
	authz "github.com/NVIDIA/infra-controller/rest-api/auth/pkg/authorization"
	"github.com/NVIDIA/infra-controller/rest-api/common/pkg/grpcproxy"
	cdbm "github.com/NVIDIA/infra-controller/rest-api/db/pkg/db/model"
	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
	swe "github.com/NVIDIA/infra-controller/rest-api/site-workflow/pkg/error"
)

const testBiosProfileBody = `{
	"vendor": "Lenovo",
	"model": "ThinkSystem SR680a V3",
	"attributeRegistry": "BiosAttributeRegistry.1.0.0",
	"attributes": {"OperatingModes_ChooseOperatingMode": "MaximumPerformance", "SystemRecovery_POSTWatchdogTimerValue": 5},
	"revision": "3f2a9c1d7b4e"
}`

const testBiosProfileJSON = `{
	"name": "lenovo-h200",
	"vendor": "Lenovo",
	"model": "ThinkSystem SR680a V3",
	"attributeRegistry": "BiosAttributeRegistry.1.0.0",
	"attributes": {"OperatingModes_ChooseOperatingMode": "MaximumPerformance", "SystemRecovery_POSTWatchdogTimerValue": 5},
	"revision": "3f2a9c1d7b4e",
	"updatedAt": "2026-10-26T04:00:00Z"
}`

func testBiosProfile() *corev1.BiosProfile {
	return &corev1.BiosProfile{
		Name:              "lenovo-h200",
		Vendor:            "Lenovo",
		Model:             "ThinkSystem SR680a V3",
		AttributeRegistry: proto.String("BiosAttributeRegistry.1.0.0"),
		AttributesJson:    `{"OperatingModes_ChooseOperatingMode":"MaximumPerformance","SystemRecovery_POSTWatchdogTimerValue":5}`,
		Revision:          "3f2a9c1d7b4e",
		UpdatedAt:         timestamppb.New(time.Date(2026, 10, 26, 4, 0, 0, 0, time.UTC)),
	}
}

func biosProfileRequest(t *testing.T, f common.TestSetupProviderMachineHandlerFixture, handler echo.HandlerFunc, method, name, siteID, body string) *httptest.ResponseRecorder {
	t.Helper()

	target := "/"
	if siteID != "" {
		target += "?" + url.Values{"siteId": []string{siteID}}.Encode()
	}
	req := httptest.NewRequest(method, target, strings.NewReader(body))
	if body != "" {
		req.Header.Set(echo.HeaderContentType, echo.MIMEApplicationJSON)
	}
	rec := httptest.NewRecorder()
	ec := echo.New().NewContext(req, rec)
	ec.SetParamNames("orgName", "name")
	ec.SetParamValues(f.Org, name)
	ec.Set("user", f.User)

	require.NoError(t, handler(ec))
	return rec
}

func TestBiosProfileHandlersProxyToCore(t *testing.T) {
	tests := []struct {
		name         string
		method       string
		profile      string
		body         string
		fullMethod   string
		response     proto.Message
		handler      func(common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc
		expectStatus int
		expectJSON   string
		checkRequest func(t *testing.T, requestJSON []byte)
	}{
		{
			name:       "put stores the profile and returns it with its update time",
			method:     http.MethodPut,
			profile:    "lenovo-h200",
			body:       testBiosProfileBody,
			fullMethod: corev1.Forge_UpsertBiosProfile_FullMethodName,
			response:   testBiosProfile(),
			handler: func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
				return NewUpsertBiosProfileHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
			},
			expectStatus: http.StatusOK,
			expectJSON:   testBiosProfileJSON,
			checkRequest: func(t *testing.T, requestJSON []byte) {
				var coreReq corev1.BiosProfile
				require.NoError(t, protojson.Unmarshal(requestJSON, &coreReq))
				assert.Equal(t, "lenovo-h200", coreReq.GetName())
				assert.Equal(t, "Lenovo", coreReq.GetVendor())
				assert.Equal(t, "ThinkSystem SR680a V3", coreReq.GetModel())
				assert.Equal(t, "BiosAttributeRegistry.1.0.0", coreReq.GetAttributeRegistry())
				assert.Equal(t, "3f2a9c1d7b4e", coreReq.GetRevision())
				assert.JSONEq(t, `{"OperatingModes_ChooseOperatingMode": "MaximumPerformance", "SystemRecovery_POSTWatchdogTimerValue": 5}`, coreReq.GetAttributesJson())
			},
		},
		{
			name:       "get lists the stored profiles",
			method:     http.MethodGet,
			fullMethod: corev1.Forge_ListBiosProfiles_FullMethodName,
			response:   &corev1.BiosProfileList{Profiles: []*corev1.BiosProfile{testBiosProfile()}},
			handler: func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
				return NewGetAllBiosProfileHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
			},
			expectStatus: http.StatusOK,
			expectJSON:   `{"profiles": [` + testBiosProfileJSON + `]}`,
		},
		{
			name:       "delete removes the profile",
			method:     http.MethodDelete,
			profile:    "lenovo-h200",
			fullMethod: corev1.Forge_DeleteBiosProfile_FullMethodName,
			handler: func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
				return NewDeleteBiosProfileHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
			},
			expectStatus: http.StatusNoContent,
			checkRequest: func(t *testing.T, requestJSON []byte) {
				var coreReq corev1.BiosProfileName
				require.NoError(t, protojson.Unmarshal(requestJSON, &coreReq))
				assert.Equal(t, "lenovo-h200", coreReq.GetName())
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			fixture := common.NewTestSetupProviderMachineHandlerFixture(t, tt.response)

			rec := biosProfileRequest(t, fixture, tt.handler(fixture), tt.method, tt.profile, fixture.SiteID, tt.body)
			require.Equal(t, tt.expectStatus, rec.Code, rec.Body.String())
			assert.Equal(t, tt.fullMethod, fixture.ProxiedReq.FullMethod)
			assert.Empty(t, fixture.ProxiedReq.EncryptedSecrets)
			if tt.expectJSON != "" {
				assert.JSONEq(t, tt.expectJSON, rec.Body.String())
			} else {
				assert.Empty(t, rec.Body.String())
			}
			if tt.checkRequest != nil {
				tt.checkRequest(t, fixture.ProxiedReq.RequestJSON)
			}
		})
	}
}

func TestBiosProfileHandlersRejectRequestsBeforeCore(t *testing.T) {
	put := func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
		return NewUpsertBiosProfileHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
	}
	list := func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
		return NewGetAllBiosProfileHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
	}
	remove := func(f common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc {
		return NewDeleteBiosProfileHandler(f.DBSession, f.SiteClientPool, f.Config).Handle
	}
	viewer := func(f *common.TestSetupProviderMachineHandlerFixture) {
		f.User = &cdbm.User{OrgData: cdbm.OrgData{f.Org: cdbm.Org{
			Name:  f.Org,
			Roles: []string{authz.ProviderViewerRole},
		}}}
	}
	tests := []struct {
		name    string
		method  string
		handler func(common.TestSetupProviderMachineHandlerFixture) echo.HandlerFunc
		profile string
		siteID  func(common.TestSetupProviderMachineHandlerFixture) string
		body    string
		mutate  func(*common.TestSetupProviderMachineHandlerFixture)
		expect  int
	}{
		{
			name:    "put without a site",
			method:  http.MethodPut,
			handler: put,
			profile: "lenovo-h200",
			siteID:  func(common.TestSetupProviderMachineHandlerFixture) string { return "" },
			body:    testBiosProfileBody,
			expect:  http.StatusBadRequest,
		},
		{
			name:    "put with a non-scalar attribute",
			method:  http.MethodPut,
			handler: put,
			profile: "lenovo-h200",
			body:    `{"vendor": "Lenovo", "model": "ThinkSystem SR680a V3", "attributes": {"A": null}}`,
			expect:  http.StatusBadRequest,
		},
		{
			name:    "put without a vendor",
			method:  http.MethodPut,
			handler: put,
			profile: "lenovo-h200",
			body:    `{"model": "ThinkSystem SR680a V3", "attributes": {}}`,
			expect:  http.StatusBadRequest,
		},
		{
			name:    "put by a provider viewer",
			method:  http.MethodPut,
			handler: put,
			profile: "lenovo-h200",
			body:    testBiosProfileBody,
			mutate:  viewer,
			expect:  http.StatusForbidden,
		},
		{
			name:    "list for an unknown site",
			method:  http.MethodGet,
			handler: list,
			siteID: func(common.TestSetupProviderMachineHandlerFixture) string {
				return "8e0b0c0e-8a43-4a3d-9a43-6f3c3f6f0a11"
			},
			expect: http.StatusBadRequest,
		},
		{
			name:    "list by a provider viewer",
			method:  http.MethodGet,
			handler: list,
			mutate:  viewer,
			expect:  http.StatusForbidden,
		},
		{
			name:    "delete without a site",
			method:  http.MethodDelete,
			handler: remove,
			profile: "lenovo-h200",
			siteID:  func(common.TestSetupProviderMachineHandlerFixture) string { return "" },
			expect:  http.StatusBadRequest,
		},
		{
			name:    "delete without a name",
			method:  http.MethodDelete,
			handler: remove,
			expect:  http.StatusBadRequest,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			fixture := common.NewTestSetupProviderMachineHandlerFixture(t, nil)
			if tt.mutate != nil {
				tt.mutate(&fixture)
			}
			siteID := fixture.SiteID
			if tt.siteID != nil {
				siteID = tt.siteID(fixture)
			}

			rec := biosProfileRequest(t, fixture, tt.handler(fixture), tt.method, tt.profile, siteID, tt.body)
			assert.Equal(t, tt.expect, rec.Code, rec.Body.String())
			assert.Empty(t, fixture.ProxiedReq.FullMethod)
		})
	}
}

func TestApplyMachineBiosHandler(t *testing.T) {
	t.Run("accepted applies return 202", func(t *testing.T) {
		fixture := common.NewTestSetupProviderMachineHandlerFixture(t, &corev1.ApplyMachineBiosResponse{Accepted: true})

		rec := fixture.Request(t, NewApplyMachineBiosHandler(fixture.DBSession, fixture.SiteClientPool, fixture.Config).Handle, http.MethodPost, "/", nil, "")
		require.Equal(t, http.StatusAccepted, rec.Code, rec.Body.String())
		assert.JSONEq(t, `{"accepted": true}`, rec.Body.String())
		assert.Equal(t, corev1.Forge_ApplyMachineBios_FullMethodName, fixture.ProxiedReq.FullMethod)
		var coreReq corev1.MachineId
		require.NoError(t, protojson.Unmarshal(fixture.ProxiedReq.RequestJSON, &coreReq))
		assert.Equal(t, fixture.MachineID, coreReq.GetId())
	})

	t.Run("a Core precondition failure is a conflict carrying the Core message", func(t *testing.T) {
		fixture := common.NewTestSetupProviderMachineHandlerFixture(t, nil)
		message := "machine fm100ht038bg3qsho433vkg684heguv282qaggmrsh2ugn1qk096n2c6hcg is not in maintenance; put it in maintenance before applying its BIOS"
		wrun := &tmocks.WorkflowRun{}
		wrun.On("Get", mock.Anything, mock.Anything).Return(tp.NewNonRetryableApplicationError("rpc error: code = FailedPrecondition desc = "+message, swe.ErrTypeNICoFailedPrecondition, nil))
		tsc := &tmocks.Client{}
		tsc.On("ExecuteWorkflow", mock.Anything, mock.Anything, grpcproxy.Core.WorkflowName, mock.MatchedBy(func(req grpcproxy.Request) bool {
			*fixture.ProxiedReq = req
			return true
		})).Return(wrun, nil)
		fixture.SiteClientPool.IDClientMap[fixture.SiteID] = tsc

		rec := fixture.Request(t, NewApplyMachineBiosHandler(fixture.DBSession, fixture.SiteClientPool, fixture.Config).Handle, http.MethodPost, "/", nil, "")
		require.Equal(t, http.StatusConflict, rec.Code, rec.Body.String())
		assert.Contains(t, rec.Body.String(), `"message":"`+message+`"`)
		assert.Equal(t, corev1.Forge_ApplyMachineBios_FullMethodName, fixture.ProxiedReq.FullMethod)
	})

	t.Run("a provider viewer cannot apply", func(t *testing.T) {
		fixture := common.NewTestSetupProviderMachineHandlerFixture(t, nil)
		fixture.User = &cdbm.User{OrgData: cdbm.OrgData{fixture.Org: cdbm.Org{
			Name:  fixture.Org,
			Roles: []string{authz.ProviderViewerRole},
		}}}

		rec := fixture.Request(t, NewApplyMachineBiosHandler(fixture.DBSession, fixture.SiteClientPool, fixture.Config).Handle, http.MethodPost, "/", nil, "")
		assert.Equal(t, http.StatusForbidden, rec.Code)
		assert.Empty(t, fixture.ProxiedReq.FullMethod)
	})
}

func TestCoreStatusMessage(t *testing.T) {
	tests := map[string]string{
		"rpc error: code = FailedPrecondition desc = machine m is not in maintenance (type: NICoFailedPrecondition, retryable: false)": "machine m is not in maintenance",
		"machine m is not in maintenance": "machine m is not in maintenance",
	}
	for input, expect := range tests {
		assert.Equal(t, expect, coreStatusMessage(input))
	}
}
