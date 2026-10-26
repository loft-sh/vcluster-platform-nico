// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package handler

import (
	"context"
	"net/http"

	"github.com/labstack/echo/v4"
	"github.com/rs/zerolog"
	tclient "go.temporal.io/sdk/client"

	"github.com/NVIDIA/infra-controller/rest-api/api/internal/config"
	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/handler/util/common"
	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/model"
	sc "github.com/NVIDIA/infra-controller/rest-api/api/pkg/client/site"
	cutil "github.com/NVIDIA/infra-controller/rest-api/common/pkg/util"
	cdb "github.com/NVIDIA/infra-controller/rest-api/db/pkg/db"
	cdbm "github.com/NVIDIA/infra-controller/rest-api/db/pkg/db/model"
	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
)

type UpsertBiosProfileHandler struct {
	dbSession *cdb.Session
	scp       *sc.ClientPool
}

func NewUpsertBiosProfileHandler(dbSession *cdb.Session, scp *sc.ClientPool, _ *config.Config) UpsertBiosProfileHandler {
	return UpsertBiosProfileHandler{
		dbSession: dbSession,
		scp:       scp,
	}
}

func (h UpsertBiosProfileHandler) Handle(c echo.Context) error {
	org, dbUser, ctx, logger, handlerSpan := common.SetupHandler("BiosProfile", "Upsert", c)
	if handlerSpan != nil {
		defer handlerSpan.End()
	}

	name := c.Param("name")
	if name == "" {
		return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, "BIOS profile name was not specified in URL", nil)
	}

	var apiReq model.APIBiosProfileRequest
	if err := c.Bind(&apiReq); err != nil {
		return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, "Failed to parse request data, potentially invalid structure", nil)
	}
	if err := apiReq.Validate(); err != nil {
		return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, err.Error(), nil)
	}
	coreReq, err := apiReq.ToProto(name)
	if err != nil {
		return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, err.Error(), nil)
	}

	stc, siteID, apiErr := authorizeBiosProfileSite(ctx, c, logger, h.dbSession, h.scp, org, dbUser)
	if apiErr != nil {
		return cutil.NewAPIErrorResponse(c, apiErr.Code, apiErr.Message, apiErr.Data)
	}

	logger.Info().Str("bios_profile", name).Str("site_id", siteID).Msg("Storing BIOS profile via Core gRPC proxy")

	coreResp := &corev1.BiosProfile{}
	if apiErr := common.ExecuteCoreGRPC(ctx, stc, corev1.Forge_UpsertBiosProfile_FullMethodName, coreReq, coreResp, siteID); apiErr != nil {
		logAPIError(logger, apiErr, "Failed to store BIOS profile via Core gRPC proxy")
		return cutil.NewAPIErrorResponse(c, apiErr.Code, apiErr.Message, nil)
	}

	apiResp, err := model.NewAPIBiosProfile(coreResp)
	if err != nil {
		return cutil.NewAPIErrorResponse(c, http.StatusInternalServerError, "Failed to decode BIOS profile returned by Core", nil)
	}
	return c.JSON(http.StatusOK, apiResp)
}

type GetAllBiosProfileHandler struct {
	dbSession *cdb.Session
	scp       *sc.ClientPool
}

func NewGetAllBiosProfileHandler(dbSession *cdb.Session, scp *sc.ClientPool, _ *config.Config) GetAllBiosProfileHandler {
	return GetAllBiosProfileHandler{
		dbSession: dbSession,
		scp:       scp,
	}
}

func (h GetAllBiosProfileHandler) Handle(c echo.Context) error {
	org, dbUser, ctx, logger, handlerSpan := common.SetupHandler("BiosProfile", "GetAll", c)
	if handlerSpan != nil {
		defer handlerSpan.End()
	}

	stc, siteID, apiErr := authorizeBiosProfileSite(ctx, c, logger, h.dbSession, h.scp, org, dbUser)
	if apiErr != nil {
		return cutil.NewAPIErrorResponse(c, apiErr.Code, apiErr.Message, apiErr.Data)
	}

	coreResp := &corev1.BiosProfileList{}
	if apiErr := common.ExecuteCoreGRPC(ctx, stc, corev1.Forge_ListBiosProfiles_FullMethodName, &corev1.ListBiosProfilesRequest{}, coreResp, siteID); apiErr != nil {
		logAPIError(logger, apiErr, "Failed to list BIOS profiles via Core gRPC proxy")
		return cutil.NewAPIErrorResponse(c, apiErr.Code, apiErr.Message, nil)
	}

	apiResp, err := model.NewAPIBiosProfileList(coreResp)
	if err != nil {
		return cutil.NewAPIErrorResponse(c, http.StatusInternalServerError, "Failed to decode BIOS profiles returned by Core", nil)
	}
	return c.JSON(http.StatusOK, apiResp)
}

type DeleteBiosProfileHandler struct {
	dbSession *cdb.Session
	scp       *sc.ClientPool
}

func NewDeleteBiosProfileHandler(dbSession *cdb.Session, scp *sc.ClientPool, _ *config.Config) DeleteBiosProfileHandler {
	return DeleteBiosProfileHandler{
		dbSession: dbSession,
		scp:       scp,
	}
}

func (h DeleteBiosProfileHandler) Handle(c echo.Context) error {
	org, dbUser, ctx, logger, handlerSpan := common.SetupHandler("BiosProfile", "Delete", c)
	if handlerSpan != nil {
		defer handlerSpan.End()
	}

	name := c.Param("name")
	if name == "" {
		return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, "BIOS profile name was not specified in URL", nil)
	}

	stc, siteID, apiErr := authorizeBiosProfileSite(ctx, c, logger, h.dbSession, h.scp, org, dbUser)
	if apiErr != nil {
		return cutil.NewAPIErrorResponse(c, apiErr.Code, apiErr.Message, apiErr.Data)
	}

	logger.Info().Str("bios_profile", name).Str("site_id", siteID).Msg("Deleting BIOS profile via Core gRPC proxy")

	if apiErr := common.ExecuteCoreGRPC(ctx, stc, corev1.Forge_DeleteBiosProfile_FullMethodName, &corev1.BiosProfileName{Name: name}, nil, siteID); apiErr != nil {
		logAPIError(logger, apiErr, "Failed to delete BIOS profile via Core gRPC proxy")
		return cutil.NewAPIErrorResponse(c, apiErr.Code, apiErr.Message, nil)
	}

	return c.NoContent(http.StatusNoContent)
}

func authorizeBiosProfileSite(ctx context.Context, c echo.Context, logger zerolog.Logger, dbSession *cdb.Session, scp *sc.ClientPool, org string, dbUser *cdbm.User) (tclient.Client, string, *cutil.APIError) {
	siteID := c.QueryParam("siteId")
	if siteID == "" {
		return nil, "", cutil.NewAPIError(http.StatusBadRequest, "siteId query parameter is required", nil)
	}
	return common.AuthorizeProviderSiteForCore(common.AuthorizeProviderSiteForCoreInput{
		Ctx:       ctx,
		Logger:    logger,
		DBSession: dbSession,
		SCP:       scp,
		Org:       org,
		User:      dbUser,
		SiteID:    siteID,
	})
}
