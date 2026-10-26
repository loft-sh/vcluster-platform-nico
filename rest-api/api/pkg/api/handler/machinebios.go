// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package handler

import (
	"errors"
	"fmt"
	"net/http"

	"github.com/labstack/echo/v4"
	"google.golang.org/protobuf/proto"

	"github.com/NVIDIA/infra-controller/rest-api/api/internal/config"
	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/handler/util/common"
	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/model"
	sc "github.com/NVIDIA/infra-controller/rest-api/api/pkg/client/site"
	auth "github.com/NVIDIA/infra-controller/rest-api/auth/pkg/authorization"
	cutil "github.com/NVIDIA/infra-controller/rest-api/common/pkg/util"
	cdb "github.com/NVIDIA/infra-controller/rest-api/db/pkg/db"
	cdbm "github.com/NVIDIA/infra-controller/rest-api/db/pkg/db/model"
	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
)

type GetMachineBiosHandler struct {
	dbSession *cdb.Session
	scp       *sc.ClientPool
}

func NewGetMachineBiosHandler(dbSession *cdb.Session, scp *sc.ClientPool, _ *config.Config) GetMachineBiosHandler {
	return GetMachineBiosHandler{
		dbSession: dbSession,
		scp:       scp,
	}
}

func (h GetMachineBiosHandler) Handle(c echo.Context) error {
	coreResp := &corev1.MachineBios{}
	machineID, apiErr := proxyMachineBiosRequest(c, h.dbSession, h.scp, "Get", corev1.Forge_GetMachineBios_FullMethodName, coreResp)
	if apiErr != nil {
		return apiErr.Send(c)
	}
	return sendMachineBios(c, machineID, coreResp)
}

type RefreshMachineBiosHandler struct {
	dbSession *cdb.Session
	scp       *sc.ClientPool
}

func NewRefreshMachineBiosHandler(dbSession *cdb.Session, scp *sc.ClientPool, _ *config.Config) RefreshMachineBiosHandler {
	return RefreshMachineBiosHandler{
		dbSession: dbSession,
		scp:       scp,
	}
}

func (h RefreshMachineBiosHandler) Handle(c echo.Context) error {
	coreResp := &corev1.MachineBios{}
	machineID, apiErr := proxyMachineBiosRequest(c, h.dbSession, h.scp, "Refresh", corev1.Forge_RefreshMachineBios_FullMethodName, coreResp)
	if apiErr != nil {
		return apiErr.Send(c)
	}
	return sendMachineBios(c, machineID, coreResp)
}

type GetMachineBiosAttributesHandler struct {
	dbSession *cdb.Session
	scp       *sc.ClientPool
}

func NewGetMachineBiosAttributesHandler(dbSession *cdb.Session, scp *sc.ClientPool, _ *config.Config) GetMachineBiosAttributesHandler {
	return GetMachineBiosAttributesHandler{
		dbSession: dbSession,
		scp:       scp,
	}
}

func (h GetMachineBiosAttributesHandler) Handle(c echo.Context) error {
	coreResp := &corev1.MachineBiosAttributes{}
	_, apiErr := proxyMachineBiosRequest(c, h.dbSession, h.scp, "GetAttributes", corev1.Forge_GetMachineBiosAttributes_FullMethodName, coreResp)
	if apiErr != nil {
		return apiErr.Send(c)
	}
	return c.JSON(http.StatusOK, model.NewAPIMachineBiosAttributes(coreResp))
}

func sendMachineBios(c echo.Context, machineID string, coreResp *corev1.MachineBios) error {
	apiResp, err := model.NewAPIMachineBios(machineID, coreResp)
	if err != nil {
		return cutil.NewAPIErrorResponse(c, http.StatusInternalServerError, "Failed to decode Machine BIOS returned by Core", nil)
	}
	return c.JSON(http.StatusOK, apiResp)
}

func proxyMachineBiosRequest(c echo.Context, dbSession *cdb.Session, scp *sc.ClientPool, handlerName string, fullMethod string, coreResp proto.Message) (string, *cutil.APIError) {
	org, dbUser, ctx, logger, handlerSpan := common.SetupHandler("MachineBios", handlerName, c)
	if handlerSpan != nil {
		defer handlerSpan.End()
	}

	if dbUser == nil {
		logger.Error().Msg("Invalid User object found in request context")
		return "", cutil.NewAPIError(http.StatusInternalServerError, "Failed to retrieve current user", nil)
	}

	ok, err := auth.ValidateOrgMembership(dbUser, org)
	if !ok {
		if err != nil {
			logger.Error().Err(err).Msg("Error validating org membership for User in request")
		} else {
			logger.Warn().Msg("Could not validate org membership for user, access denied")
		}
		return "", cutil.NewAPIError(http.StatusForbidden, fmt.Sprintf("Failed to validate membership for org: %s", org), nil)
	}

	machineID := c.Param("id")
	if machineID == "" {
		return "", cutil.NewAPIError(http.StatusBadRequest, "Machine ID was not specified in URL", nil)
	}

	provider, _, apiError := common.IsProviderOrTenant(ctx, logger, dbSession, org, dbUser, false, nil)
	if apiError != nil {
		return "", apiError
	}

	if provider == nil {
		logger.Warn().Msg("user does not have Provider role, access denied")
		return "", cutil.NewAPIError(http.StatusForbidden, "User does not have Provider Admin role with org", nil)
	}

	machine, err := cdbm.NewMachineDAO(dbSession).GetByID(ctx, nil, machineID, []string{cdbm.SiteRelationName}, false)
	if err != nil {
		if errors.Is(err, cdb.ErrDoesNotExist) {
			return "", cutil.NewAPIError(http.StatusNotFound, "Could not find Machine with specified ID", nil)
		}
		logger.Error().Err(err).Msg("failed to retrieve Machine details from DB")
		return "", cutil.NewAPIError(http.StatusInternalServerError, "Failed to retrieve Machine details, DB error", nil)
	}

	if machine.InfrastructureProviderID != provider.ID {
		logger.Error().Msg("Machine doesn't belong to org's Infrastructure provider")
		return "", cutil.NewAPIError(http.StatusNotFound, "Could not find Machine with specified ID", nil)
	}

	if machine.IsMissingOnSite {
		logger.Error().Msg("Machine is missing on site, unable to read its BIOS")
		return "", cutil.NewAPIError(http.StatusBadRequest, "Machine is missing on site, unable to read its BIOS", nil)
	}

	if machine.Site == nil {
		logger.Error().Msg("Related Site was not returned for Machine DB entity")
		return "", cutil.NewAPIError(http.StatusInternalServerError, "Failed to retrieve Site details for Machine, DB error", nil)
	}

	site := machine.Site

	if site.Status != cdbm.SiteStatusRegistered {
		logger.Warn().Msg("Site specified in request data is not in Registered state")
		return "", cutil.NewAPIError(http.StatusBadRequest, "Site specified in request data is not in Registered state, cannot execute admin operation", nil)
	}

	stc, err := scp.GetClientByID(site.ID)
	if err != nil {
		logger.Error().Err(err).Msg("failed to retrieve Temporal client for Site")
		return "", cutil.NewAPIError(http.StatusInternalServerError, "Failed to retrieve workflow client for Site", nil)
	}

	logger.Info().Str("machine_id", machineID).Str("site_id", site.ID.String()).Str("method", fullMethod).Msg("Sending Machine BIOS request via Core gRPC proxy")

	apiErr := common.ExecuteCoreGRPC(ctx, stc, fullMethod, &corev1.MachineId{Id: machineID}, coreResp, site.ID.String())
	if apiErr != nil {
		logAPIError(logger, apiErr, "Failed to execute Machine BIOS request via Core gRPC proxy")
		return "", cutil.NewAPIError(apiErr.Code, apiErr.Message, nil)
	}

	return machineID, nil
}
