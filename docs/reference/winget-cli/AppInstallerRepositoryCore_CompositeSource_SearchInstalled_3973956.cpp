// Excerpts from microsoft/winget-cli @ 39739564a4aaf1071b17d163ec332a08b9bcf05c (MIT, see LICENSE).
// Relevant to wingetrs Task 21 (tie-breaks for ambiguous installed correlations).
// Summary: available-source search by system references; >1 match => only a single STRONG
// (ProductCode/UpgradeCode/PFN) match is accepted, otherwise indeterminate; then the tracking
// catalog (installed.db, packages installed by winget) supplies the id, latest TrackingWriteTime wins.

// ===== src/AppInstallerRepositoryCore/CompositeSource.cpp lines 23-137 =====
        // Returns true for fields that provide a strong match; one that is not based on a heuristic.
        bool IsStrongMatchField(PackageMatchField field)
        {
            switch (field)
            {
            case AppInstaller::Repository::PackageMatchField::PackageFamilyName:
            case AppInstaller::Repository::PackageMatchField::ProductCode:
            case AppInstaller::Repository::PackageMatchField::UpgradeCode:
                return true;
            }

            return false;
        }

        // Gets the only available package from the composite, ensuring this fact in test contexts.
        std::shared_ptr<IPackage> OnlyAvailable(const std::shared_ptr<ICompositePackage>& composite)
        {
            std::vector<std::shared_ptr<IPackage>> availablePackages = composite->GetAvailable();

#ifndef AICLI_DISABLE_TEST_HOOKS
            THROW_HR_IF(E_UNEXPECTED, availablePackages.size() != 1);
#endif

            return std::move(availablePackages.front());
        }

        // Move returns if there is only one package in the matches that is strong; otherwise returns an empty value.
        std::shared_ptr<ICompositePackage> FindOnlyStrongMatchFieldResult(std::vector<ResultMatch>& matches)
        {
            std::shared_ptr<ICompositePackage> result;

            for (auto&& match : matches)
            {
                AICLI_LOG(Repo, Info, << "  Checking match with package id: " << match.Package->GetProperty(PackageProperty::Id));

                if (IsStrongMatchField(match.MatchCriteria.Field))
                {
                    if (!result)
                    {
                        result = std::move(match.Package);
                    }
                    else
                    {
                        AICLI_LOG(Repo, Info, << "  Found multiple packages with strong match fields");
                        result.reset();
                        break;
                    }
                }
            }

            return result;
        }

        // Gets a single matching package from the results
        template <typename MultipleIntro, typename Indeterminate>
        std::shared_ptr<ICompositePackage> GetMatchingPackage(std::vector<ResultMatch>& matches, MultipleIntro&& multipleIntro, Indeterminate&& indeterminate)
        {
            if (matches.empty())
            {
                return {};
            }
            else if (matches.size() == 1)
            {
                return std::move(matches[0].Package);
            }
            else
            {
                multipleIntro();

                auto result = FindOnlyStrongMatchFieldResult(matches);

                if (!result)
                {
                    indeterminate();
                }

                return result;
            }
        }

        // For a given package from a tracking catalog, get the latest write time.
        // Look at all versions rather than just the latest to account for the potential of downgrading.
        std::chrono::system_clock::time_point GetLatestTrackingWriteTime(
            const std::shared_ptr<IPackage>& trackingPackage)
        {
            std::chrono::system_clock::time_point result{};

            for (const auto& key : trackingPackage->GetVersionKeys())
            {
                auto version = trackingPackage->GetVersion(key);
                if (version)
                {
                    auto metadata = version->GetMetadata();
                    auto itr = metadata.find(PackageVersionMetadata::TrackingWriteTime);
                    if (itr != metadata.end())
                    {
                        std::int64_t unixEpoch = 0;
                        try
                        {
                            unixEpoch = std::stoll(itr->second);
                        }
                        CATCH_LOG();

                        std::chrono::system_clock::time_point versionTime = Utility::ConvertUnixEpochToSystemClock(unixEpoch);

                        if (versionTime > result)
                        {
                            result = versionTime;
                        }
                    }
                }
            }

            return result;
        }

// ===== src/AppInstallerRepositoryCore/CompositeSource.cpp lines 1442-1585 =====
    // An installed search first finds all installed packages that match the request, then correlates with available sources.
    // Next the search is performed against the available sources and correlated with the installed source. A result will only
    // be added if there exists an installed package that was not found by the initial search.
    // This allows for search terms to find installed packages by their available metadata, as well as the local values.
    //
    // Search flow:
    //  Installed :: Search incoming request
    //  For each result
    //      For each available source
    //          Tracking :: Search system references
    //      If tracking found
    //          Available :: Search tracking ID
    //      If no available, for each available source
    //          Available :: Search system references
    // 
    //  For each available source
    //      Tracking :: Search incoming request
    //      For each result
    //          Installed :: Search system references
    //          If found
    //              Available :: Search tracking ID
    //      Available :: Search incoming request
    //      For each result
    //          Installed :: Search system references
    SearchResult CompositeSource::SearchInstalled(const SearchRequest& request) const
    {
        CompositeResult result;

        // If the search behavior is for AllPackages or Installed then the result can contain packages that are
        // only in the Installed source, but do not have an AvailableVersion.
        if (m_searchBehavior == CompositeSearchBehavior::AllPackages || m_searchBehavior == CompositeSearchBehavior::Installed)
        {
            // Search installed source (allow exceptions out as we own the installed source)
            SearchResult installedResult = m_installedSource.Search(request);
            result.Truncated = installedResult.Truncated;

            for (auto&& match : installedResult.Matches)
            {
                if (!match.Package)
                {
                    // Ensure that the crash from installedVersion below is not from the actual package being null.
                    AICLI_LOG(Repo, Warning, << "CompositeSource: The match of the package (matched on " <<
                        ToString(match.MatchCriteria.Field) << " => '" << match.MatchCriteria.Value <<
                        "') was null and is being dropped from the results.");
                    continue;
                }

                std::shared_ptr<CompositePackage> compositePackage = std::make_shared<CompositePackage>(match.Package);
                auto installedPackage = compositePackage->GetInstalled();

                if (!installedPackage)
                {
                    // One would think that the installed package coming directly from our own installed source
                    // would never be null, but it is sometimes. Rather than making users suffer through crashes
                    // that break their entire experience, lets log a few things and then ignore this match.
                    AICLI_LOG(Repo, Warning, << "CompositeSource: The installed version of the package '" <<
                        match.Package->GetProperty(PackageProperty::Id) << "' was null and is being dropped from the results.");
                    continue;
                }

                auto installedPackageData = result.GetSystemReferenceStrings(installedPackage.get());

                // Create a search request to run against all available sources
                if (!installedPackageData.SystemReferenceStrings.empty())
                {
                    SearchRequest systemReferenceSearch = installedPackageData.CreateInclusionsSearchRequest(SearchPurpose::CorrelationToAvailable);
                    AICLI_LOG(Repo, Verbose, << "Finding available package from installed package using system reference search: " << systemReferenceSearch.ToString());

                    // Search sources and add to result
                    for (const auto& source : m_availableSources)
                    {
                        AICLI_LOG(Repo, Verbose, << " ... searching source: " << source.GetDetails().Name << " [" << source.GetIdentifier() << ']');

                        // Find the tracking result with the latest timestamp.
                        auto trackingCatalog = source.GetTrackingCatalog();
                        SearchResult trackingResult = trackingCatalog.Search(systemReferenceSearch);

                        std::shared_ptr<IPackage> trackingPackage;
                        std::chrono::system_clock::time_point trackingPackageTime;
                        bool trackingSet = false;

                        for (const auto& trackingMatch : trackingResult.Matches)
                        {
                            auto candidateTime = GetLatestTrackingWriteTime(OnlyAvailable(trackingMatch.Package));

                            if (!trackingPackage || candidateTime > trackingPackageTime)
                            {
                                trackingPackage = OnlyAvailable(trackingMatch.Package);
                                trackingPackageTime = candidateTime;
                            }
                        }

                        if (trackingPackage && trackingPackageTime > compositePackage->GetTrackingPackageWriteTime())
                        {
                            AICLI_LOG(Repo, Verbose, << " ... setting latest tracking package to: " << trackingPackage->GetProperty(PackageProperty::Id));
                            compositePackage->SetTracking(source, trackingPackage, trackingPackageTime);
                            trackingSet = true;
                        }

                        // Attempt to correlate local packages against this source if supported.
                        SearchResult availableResult;
                        if (source.GetDetails().SupportInstalledSearchCorrelation)
                        {
                            availableResult = result.SearchAndHandleFailures(source, systemReferenceSearch);
                        }

                        auto availablePackage = GetMatchingPackage(availableResult.Matches,
                            [&]() {
                                AICLI_LOG(Repo, Info,
                                << "Found multiple matches for installed package [" << installedPackage->GetProperty(PackageProperty::Id) <<
                                "] in source [" << source.GetIdentifier() << "] when searching for [" << systemReferenceSearch.ToString() << "]");
                            }, [&] {
                                AICLI_LOG(Repo, Warning, << "  Appropriate available package could not be determined");
                            });

                        if (trackingPackage)
                        {
                            auto trackingIdentifier = trackingPackage->GetProperty(PackageProperty::Id);

                            // We always want to take the available search result if it exists as the package may have been updated.
                            if (availablePackage)
                            {
                                auto availableIdentifier = availablePackage->GetProperty(PackageProperty::Id);
                                if (!Utility::ICUCaseInsensitiveEquals(availableIdentifier, trackingIdentifier))
                                {
                                    AICLI_LOG(Repo, Verbose, << " ... overriding tracking package (" << trackingIdentifier << ") with available package (" << availableIdentifier << ")");
                                }
                            }
                            else
                            {
                                AICLI_LOG(Repo, Verbose, << " ... using tracking package: " << trackingIdentifier);
                                availablePackage = GetTrackedPackageFromAvailableSource(result, source, trackingIdentifier);
                            }
                        }

                        if (availablePackage)
                        {
                            AICLI_LOG(Repo, Verbose, << " ... adding available package: " << availablePackage->GetProperty(PackageProperty::Id));
                            compositePackage->AddAvailablePackage(availablePackage, trackingSet);
                        }
                    }
                }

                // Move the installed result into the composite result

// ===== src/AppInstallerRepositoryCore/PackageTrackingCatalog.cpp lines 17-33 (file location) =====
    namespace
    {
        constexpr std::string_view c_PackageTrackingFileName = "installed.db";
        constexpr std::string_view c_PackageTrackingCorruptedFileName = "installed-corrupted.db";

        std::string CreateNameForCPL(const std::string& pathName)
        {
            return "PackageTrackingCPL_"s + pathName;
        }

        std::filesystem::path GetPackageTrackingFilePath(const std::string& pathName)
        {
            std::filesystem::path result = Runtime::GetPathTo(Runtime::PathName::LocalState);
            result /= pathName;
            result /= c_PackageTrackingFileName;
            return result;
        }

// ===== src/AppInstallerRepositoryCore/PackageTrackingCatalog.cpp lines 160-185 (source identifier) =====
    PackageTrackingCatalog::PackageTrackingCatalog(PackageTrackingCatalog&&) noexcept = default;
    PackageTrackingCatalog& PackageTrackingCatalog::operator=(PackageTrackingCatalog&&) noexcept = default;
    PackageTrackingCatalog::~PackageTrackingCatalog() = default;

    PackageTrackingCatalog PackageTrackingCatalog::CreateForSource(const Source& source)
    {
        // Not a valid source for tracking
        const std::string sourceIdentifier = source.GetIdentifier();
        if (sourceIdentifier.empty() || !source.ContainsAvailablePackages())
        {
            THROW_HR(E_INVALIDARG);
        }

        // Create fake details for the source while stashing some information that might be helpful for debugging
        SourceDetails details;
        details.Type = PackageTrackingCatalogSourceFactory::Type();
        details.Identifier = "*Tracking";
        details.Name = "Tracking for "s + source.GetDetails().Name;
        details.Origin = SourceOrigin::PackageTracking;
        details.Data = sourceIdentifier;

        ProgressCallback dummyProgress;

        PackageTrackingCatalog result;
        result.m_implementation = std::make_shared<PackageTrackingCatalog::implementation>();
        result.m_implementation->Source = SourceCast<SQLiteIndexSource>(ISourceFactory::GetForType(details.Type)->Create(details)->Open(dummyProgress));

// ===== src/AppInstallerRepositoryCore/Public/winget/RepositorySearch.h lines 184-221 =====
    enum class PackageVersionMetadata : int32_t
    {
        // The InstallerType of an installed package
        InstalledType,
        // The Scope of an installed package
        InstalledScope,
        // The system path where the package is installed
        InstalledLocation,
        // The standard uninstall command; which may be interactive
        StandardUninstallCommand,
        // An uninstall command that should be non-interactive
        SilentUninstallCommand,
        // The publisher of the package
        Publisher,
        // The locale of the package
        InstalledLocale,
        // The write time for the given version
        TrackingWriteTime,
        // The Architecture of an installed package
        InstalledArchitecture,
        // The pinned state of the installed package
        // As a package can have multiple pins for multiple sources, this is the strictest pin
        PinnedState,
        // The Architecture of user intent
        UserIntentArchitecture,
        // The locale of user intent
        UserIntentLocale,
        // The standard modify command; which may be interactive
        StandardModifyCommand,
        // No Modify flag
        NoModify,
        // No Repair flag
        NoRepair,
        // The --override arguments provided by the user when initially installing the package; preserved on upgrade
        InitialOverrideArguments,
        // The --custom switches provided by the user when initially installing the package; preserved on upgrade
        InitialCustomSwitches,
    };
