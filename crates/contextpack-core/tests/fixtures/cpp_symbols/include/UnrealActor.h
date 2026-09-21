#pragma once

UCLASS(BlueprintType)
class PIZZERIASIMULATOR_API AStaticMeshAnimationActor : public AActor
{
    GENERATED_BODY()

public:
    AStaticMeshAnimationActor();

protected:
    UPROPERTY(EditAnywhere, Category = "Animation")
    TObjectPtr<UStaticMesh> AnimationMesh;

    UFUNCTION(BlueprintCallable, Category = "Animation")
    void StartAnimation();

    virtual void BeginPlay() override;

    FORCEINLINE int32 GetFrameCount() const
    {
        return FrameCount;
    }

private:
    int32 FrameCount = 0;
};

